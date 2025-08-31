// src/main.rs

#![windows_subsystem = "windows"]

mod tts_engine;
mod i18n;
mod event_monitor;
mod config;
mod startup;
mod settings_ui;

// --- 新增: 引入日志宏 ---
use log::{info, error, warn, debug};
use std::time::{Duration, Instant};

use std::ffi::c_void;
use std::sync::{mpsc, Arc, Mutex};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NOTIFYICONDATAW, NIM_ADD, NIM_DELETE, NIF_ICON, NIF_MESSAGE, NIF_TIP};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetWindowLongPtrW, LoadIconW, PeekMessageW, PostQuitMessage, RegisterClassW, RegisterDeviceNotificationW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA, IDI_APPLICATION, MF_STRING, PM_REMOVE, TPM_BOTTOMALIGN, WM_APP, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DEVICECHANGE, WM_POWERBROADCAST, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW
};
use windows::Win32::System::Power::{GetSystemPowerStatus, RegisterPowerSettingNotification, POWERBROADCAST_SETTING, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemServices::GUID_ACDC_POWER_SOURCE;
use windows::Win32::UI::WindowsAndMessaging::{PBT_POWERSETTINGCHANGE, REGISTER_NOTIFICATION_FLAGS, DEV_BROADCAST_DEVICEINTERFACE_W, DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_WINDOW_HANDLE, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE};
use windows::Win32::Devices::Usb::GUID_DEVINTERFACE_USB_DEVICE;
use windows::Win32::System::WindowsProgramming::GetUserNameW;
use windows::core::PWSTR;

use crate::config::Config;
use crate::event_monitor::{SystemEvent, ConnectionType, start_monitoring, IS_SYSTEM_ASLEEP};
use crate::i18n::I18nManager;
use crate::tts_engine::TtsEngine;

// --- 全域常量 ---
const WM_APP_TRAY_MSG: u32 = WM_APP + 1;
const ID_MENU_PAUSE_RESUME: u32 = 1001;
const ID_MENU_SETTINGS: u32 = 1002;
const ID_MENU_EXIT: u32 = 1003;

struct WindowProcData {
    sender: mpsc::Sender<SystemEvent>,
    app_state: Arc<Mutex<AppState>>,
}

// --- 共享狀態 ---
struct AppState {
    is_paused: bool,
    tts_engine: TtsEngine,
    i18n_manager: I18nManager,
    username: String,
    last_usb_connect_time: Option<Instant>,
    last_usb_disconnect_time: Option<Instant>,
    config: Config,
    available_voices: Vec<String>,
}

// --- 修改: main 函数返回 Result ---
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --- 日志初始化 (不变) ---
    simple_logging::log_to_file("advanced_beeper.log", log::LevelFilter::Info)?;
    info!("-----------------------------------------");
    info!("高级提示 (Advanced Beeper) 应用程式启动");
    info!("-----------------------------------------");

    // --- 1. 初始化 ---
    // 步骤 1: 首先加载配置
    let config = Config::load();
    info!("配置文件 config.json 已加载: {:?}", config);

    let effective_locale = match &config.language {
        Some(lang_code) => {
            info!("配置中指定了语言: '{}'", lang_code);
            lang_code.clone()
        },
        None => {
            info!("配置中未指定语言，尝试自动检测系统语言。");
            match sys_locale::get_locale() {
                Some(sys_lang) => {
                    info!("检测到系统语言: {}", sys_lang);
                    let lang_prefix = sys_lang.split('-').next().unwrap_or(&sys_lang);
                    match lang_prefix {
                        "zh" => "zh".to_string(),
                        "ja" => "ja".to_string(),
                        _ => {
                            info!("系统语言 '{}' 不在支持列表(zh, ja)中，回退到英语。", lang_prefix);
                            "en".to_string()
                        }
                    }
                },
                None => {
                    warn!("无法获取系统语言，回退到英语。");
                    "en".to_string()
                }
            }
        }
    };

    // 步骤 2: 根据加载的配置，同步注册表
    if let Err(e) = startup::set_auto_start(config.auto_start) {
        error!("启动时同步开机自启动设置失败: {}", e);
    }

    let (sender, receiver) = mpsc::channel();
    let locale = "en"; // 备用参数
    
    // 步骤 3: 初始化语音引擎时，传入配置
    let tts_engine = TtsEngine::new(&config).map_err(|e| {
        error!("语音引擎初始化失败: {}", e);
        e
    })?;

    // --- 新增: 在这里获取并缓存语音列表 ---
    let available_voices = match tts_engine.list_available_voices() {
        Ok(voices) => {
            info!("成功获取到 {} 个可用语音。", voices.len());
            voices
        },
        Err(e) => {
            error!("启动时获取可用语音列表失败: {}", e);
            vec![] // 如果失败，则提供一个空的列表
        }
    };

    let i18n_manager = I18nManager::new(&effective_locale).map_err(|e| {
        error!("语言档案(locale: {})载入失败: {}", &effective_locale, e);
        e
    })?;
    info!("国际化语言档案 (locale: {}) 载入成功。", effective_locale);

    // 步骤 4: 创建共享状态
    let app_state = Arc::new(Mutex::new(AppState {
        is_paused: false,
        tts_engine,
        i18n_manager,
        username: get_windows_username(),
        last_usb_connect_time: None,
        last_usb_disconnect_time: None,
        config,
        available_voices,
    }));

    info!("当前 Windows 用户名: {}", app_state.lock().unwrap().username);
    
    // --- 新增：发送启动事件 ---
    // 在所有核心服务都准备好之后，立即发送启动事件来播报欢迎语。
    info!("所有模块初始化完毕，发送系统启动事件。");
    if let Err(e) = sender.send(SystemEvent::SystemStartup) {
        error!("在启动时发送 SystemStartup 事件失败: {}", e);
    }

    // --- 2. 建立 Win32 视窗 (修改：增加了详细日志和错误检查) ---
    let window_proc_data = Box::into_raw(Box::new(WindowProcData {
        sender: sender.clone(),
        app_state: app_state.clone(),
    }));
    
    let class_name = w!("AdvancedPromptsHiddenWindowClass");
    let instance = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None)? };
    info!("获取到模块句柄: {:?}", instance);

    let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class_name, ..Default::default() };
    
    // 增加对 RegisterClassW 的返回值检查
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 {
        let last_error = windows::core::Error::from_win32();
        error!("注册窗口类失败 (RegisterClassW failed): {:?}", last_error);
        return Err(Box::new(last_error));
    }
    info!("窗口类注册成功 (Atom: {}).", atom);

    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(), class_name, w!("CO/MP/UT/ER"), WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
            None, None, Some(instance.into()), Some(window_proc_data as *mut c_void),
        )?
    };
    info!("隐藏主窗口已建立 (HWND: {:?})。", hwnd);

    // --- 3. 启动背景监控 (不变) ---
    start_monitoring(sender);
    info!("已分派背景事件监控线程。");

    // --- 4. 运行主消息循环 (不变) ---
    info!("主消息循环已启动，等待事件...");
    let mut msg = Default::default();
    loop {
        while let Ok(event) = receiver.try_recv() {
            handle_system_event(event, &app_state);
        }

        if unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
                info!("接收到 WM_QUIT 消息，准备退出。");
                break;
            }
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    
    info!("应用程序主循环结束，正常退出。");
    Ok(())
}


// --- 視窗程序 (Win32 事件處理中心) ---
extern "system" fn wndproc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_CREATE {
        // ... 这部分代码保持不变 ...
        info!("wndproc 接收到 WM_CREATE 消息。");
        let create_struct = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        let data_ptr = create_struct.lpCreateParams as *mut WindowProcData;
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, data_ptr as isize); }
        add_tray_icon(window);
        
        let _ = unsafe { RegisterPowerSettingNotification(window.into(), &GUID_ACDC_POWER_SOURCE, REGISTER_NOTIFICATION_FLAGS(0)) };
        info!("已注册电源设置 (AC/DC) 通知。");
        
        let mut filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_classguid: GUID_DEVINTERFACE_USB_DEVICE,
            ..Default::default()
        };
        let _ = unsafe { RegisterDeviceNotificationW(window.into(), &mut filter as *mut _ as *mut c_void, DEVICE_NOTIFY_WINDOW_HANDLE) };
        info!("已注册 USB 设备插拔通知。");

        return LRESULT(0);
    }

    let data_ptr = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut WindowProcData;
    if data_ptr.is_null() { return unsafe { DefWindowProcW(window, message, wparam, lparam) }; }
    
    let data = unsafe { &*data_ptr };
    let sender = &data.sender;
    let app_state_arc = &data.app_state;
    
    match message {
        WM_POWERBROADCAST => {
            if *IS_SYSTEM_ASLEEP.lock().unwrap() { return LRESULT(0); }
            
            // --- 修改: 详细解析电源事件 ---
            info!("wndproc: 接收到 WM_POWERBROADCAST 消息 (WPARAM: {:#x})", wparam.0);

            if wparam.0 as u32 == PBT_POWERSETTINGCHANGE {
                let pbs = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
                if pbs.PowerSetting == GUID_ACDC_POWER_SOURCE {
                    let source = unsafe { *(pbs.Data.as_ptr() as *const u32) };
                    let event = if source == 0 { // 0 代表交流电源 AC
                        SystemEvent::PowerSwitchedToAC
                    } else { // 1 代表电池 DC
                        SystemEvent::PowerSwitchedToBattery
                    };
                    info!("wndproc: 检测到电源切换 -> {:?}", event);
                    if let Err(e) = sender.send(event) {
                        error!("从 wndproc 发送电源切换事件失败: {}", e);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DEVICECHANGE => {
            if *IS_SYSTEM_ASLEEP.lock().unwrap() { return LRESULT(0); }
            let event_type = wparam.0 as u32;
            if event_type == DBT_DEVICEARRIVAL || event_type == DBT_DEVICEREMOVECOMPLETE {
                 let event = if event_type == DBT_DEVICEARRIVAL { SystemEvent::UsbDeviceConnected } else { SystemEvent::UsbDeviceDisconnected };
                 handle_debounced_usb_event(event, sender, app_state_arc);
            }
            LRESULT(0)
        }
        // ... 其他消息处理保持不变 ...
        WM_APP_TRAY_MSG => {
            if lparam.0 as u32 == WM_RBUTTONUP {
                debug!("wndproc: 接收到系统托盘右键点击消息。");
                show_context_menu(window);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            match wparam.0 as u32 {
                ID_MENU_PAUSE_RESUME => {
                    info!("wndproc: '暂停/恢复播报' 菜单项被点击。");
                    // TODO: 实现暂停/恢复逻辑
                }
                // --- 新增: 处理设置菜单点击 ---
                ID_MENU_SETTINGS => {
                    info!("wndproc: '设置' 菜单项被点击。");
                    // 创建并显示设置窗口
                    // 我们需要克隆 Arc 以便在新线程或窗口中使用
                    let app_state_clone = app_state_arc.clone();
                    settings_ui::show(window, app_state_clone);
                }
                ID_MENU_EXIT => {
                    info!("wndproc: '退出' 菜单项被点击。");
                    let _ = unsafe { DestroyWindow(window) };
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            info!("wndproc: 接收到 WM_DESTROY 消息。");
            remove_tray_icon(window);
            let _ = unsafe { Box::from_raw(SetWindowLongPtrW(window, GWLP_USERDATA, 0) as *mut WindowProcData) };
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

// --- 播報邏輯 ---
fn handle_system_event(event: SystemEvent, app_state_arc: &Arc<Mutex<AppState>>) {
    info!("主循环收到事件: {:?}", event);
    let mut app_state = app_state_arc.lock().unwrap();
    
    match event {
        SystemEvent::SystemGoingToSleep | SystemEvent::SystemResumedFromSleep => { /* Do nothing special, just let it pass */ }
        _ if app_state.is_paused => {
            warn!("系统当前处于暂停状态，已忽略事件: {:?}", event);
            return;
        }
        _ => {}
    }
    
    let i18n = &app_state.i18n_manager;
    
    let text_to_speak = match &event {
        SystemEvent::SystemStartup => i18n.get_text_with_param("system_online", "user", &app_state.username),
        SystemEvent::PowerSwitchedToAC => i18n.get_text("external_power_connected"),
        SystemEvent::PowerSwitchedToBattery => i18n.get_text("switched_to_battery"),
        SystemEvent::BatteryLevelReport(level) => i18n.get_text_with_param("battery_level_report", "level", &level.to_string()),
        SystemEvent::UsbDeviceConnected => i18n.get_text("usb_device_detected"),
        SystemEvent::UsbDeviceDisconnected => i18n.get_text("usb_device_disconnected"),
        SystemEvent::BatteryInserted => {
            let mut sps = SYSTEM_POWER_STATUS::default();
            if unsafe { GetSystemPowerStatus(&mut sps) }.is_ok() && sps.BatteryLifePercent != 255 {
                i18n.get_text_with_param("battery_inserted", "level", &sps.BatteryLifePercent.to_string())
            } else {
                i18n.get_text("battery_inserted").map(|s| s.replace(" Current battery level is {level} percent.", ""))
            }
        }
        SystemEvent::BatteryRemoved => i18n.get_text("battery_removed"),
        SystemEvent::NetworkConnected { name, conn_type } => match conn_type {
            ConnectionType::WiFi => i18n.get_text_with_param("network_connected_wifi", "SSID", name),
            ConnectionType::Cellular => i18n.get_text("network_connected_cellular"),
            ConnectionType::Ethernet => i18n.get_text("network_connected_ethernet"),
            ConnectionType::Unknown => i18n.get_text_with_param("network_connected_unknown", "SSID", name),
        },
        SystemEvent::NetworkDisconnected => i18n.get_text("network_disconnected"),
        SystemEvent::SystemGoingToSleep => i18n.get_text("system_going_to_sleep"),
        SystemEvent::SystemResumedFromSleep => i18n.get_text("system_resumed_from_sleep"),
    };
    
    if let Some(text) = text_to_speak {
        info!("准备播报: '{}'", text);
        if let Err(e) = app_state.tts_engine.speak(&text) {
            error!("语音播报失败: {}", e);
        } else {
            info!("语音播报成功。");
        }
    } else {
        warn!("未能为事件 {:?} 找到对应的提示语文字！", event);
    }
}

// --- 輔助函數 ---

const USB_DEBOUNCE_DURATION: Duration = Duration::from_secs(2);

fn handle_debounced_usb_event(
    event: SystemEvent, 
    sender: &mpsc::Sender<SystemEvent>, 
    app_state_arc: &Arc<Mutex<AppState>>
) {
    let mut app_state = app_state_arc.lock().unwrap();
    let now = Instant::now();

    let should_send = match event {
        SystemEvent::UsbDeviceConnected => {
            if let Some(last_time) = app_state.last_usb_connect_time {
                if now.duration_since(last_time) < USB_DEBOUNCE_DURATION {
                    info!("wndproc: 忽略重复的 USB 连接事件 (去抖)。");
                    false // 时间太近，忽略
                } else {
                    app_state.last_usb_connect_time = Some(now);
                    true // 时间足够长，发送
                }
            } else {
                app_state.last_usb_connect_time = Some(now);
                true // 第一次，发送
            }
        }
        SystemEvent::UsbDeviceDisconnected => {
            if let Some(last_time) = app_state.last_usb_disconnect_time {
                if now.duration_since(last_time) < USB_DEBOUNCE_DURATION {
                    info!("wndproc: 忽略重复的 USB 断开事件 (去抖)。");
                    false
                } else {
                    app_state.last_usb_disconnect_time = Some(now);
                    true
                }
            } else {
                app_state.last_usb_disconnect_time = Some(now);
                true
            }
        }
        _ => true, // 其他非 USB 事件直接发送
    };

    if should_send {
        info!("wndproc: 检测到有效的 USB 设备事件 -> {:?}", event);
        if let Err(e) = sender.send(event) {
            error!("从 wndproc 发送 USB 事件失败: {}", e);
        }
    }
}

fn get_windows_username() -> String {
    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;
    unsafe {
        if GetUserNameW(Some(PWSTR(buffer.as_mut_ptr())), &mut size).is_ok() {
            String::from_utf16_lossy(&buffer[..size as usize])
        } else {
            "user".to_string()
        }
    }
}

fn add_tray_icon(hwnd: HWND) {
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_APP_TRAY_MSG;
    nid.hIcon = unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or_default() };
    
    let tip = w!("CO/MP/UT/ER");
    unsafe {
        nid.szTip[..tip.len()].copy_from_slice(tip.as_wide());
    }
    
    if unsafe { Shell_NotifyIconW(NIM_ADD, &nid) }.as_bool() {
        info!("系统托盘图标添加成功。");
    } else {
        error!("系统托盘图标添加失败。");
    }
}

fn remove_tray_icon(hwnd: HWND) {
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    if unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) }.as_bool() {
        info!("系统托盘图标移除成功。");
    } else {
        warn!("系统托盘图标移除失败 (可能是因为窗口已销毁)。");
    }
}

fn show_context_menu(hwnd: HWND) {
    let menu = unsafe { CreatePopupMenu().unwrap() };
    unsafe {
        // TODO: 根據目前的 is_paused 狀態，動態修改選單文字
        let _ = AppendMenuW(menu, MF_STRING, ID_MENU_PAUSE_RESUME as usize, w!("暫停/恢復播報"));
        let _ = AppendMenuW(menu, MF_STRING, ID_MENU_SETTINGS as usize, w!("设置..."));
        let _ = AppendMenuW(menu, MF_STRING, ID_MENU_EXIT as usize, w!("退出"));
        
        let mut point = Default::default();
        let _ = GetCursorPos(&mut point);
        
        // 必須設為前景，否則選單在失去焦點時不會自動關閉
        let _ = SetForegroundWindow(hwnd);
        
        let _ = TrackPopupMenu(menu, TPM_BOTTOMALIGN, point.x, point.y, Some(0), hwnd, None);
    }
}