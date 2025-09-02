// src/main.rs

#![windows_subsystem = "windows"]

mod tts_engine;
mod i18n;
mod event_monitor;
mod config;
mod startup;
mod settings_ui;

use log::{info, error, warn, debug};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use std::time::{Duration, Instant};

use std::env;
use std::ffi::c_void;
use std::error::Error;
use std::sync::{mpsc, Arc, Mutex};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
// --- FIX: 引入 COM 初始化相关的常量 ---
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NOTIFYICONDATAW, NIM_ADD, NIM_DELETE, NIF_ICON, NIF_MESSAGE, NIF_TIP};
use windows::Win32::UI::WindowsAndMessaging::{
    DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DEV_BROADCAST_HDR, GetMessageW, MSG, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetWindowLongPtrW, LoadIconW, PostQuitMessage, RegisterClassW, RegisterDeviceNotificationW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenu, TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, GWLP_USERDATA, IDI_APPLICATION, MF_STRING, TPM_BOTTOMALIGN, TPM_LEFTALIGN, WM_APP, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_POWERBROADCAST, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPEDWINDOW, PBT_APMSUSPEND, PBT_APMRESUMEAUTOMATIC, PBT_POWERSETTINGCHANGE, REGISTER_NOTIFICATION_FLAGS, DEV_BROADCAST_DEVICEINTERFACE_W, DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_WINDOW_HANDLE, WM_DEVICECHANGE,
    PostMessageW,
};
use windows::Win32::System::Power::{GetSystemPowerStatus, RegisterPowerSettingNotification, POWERBROADCAST_SETTING, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_CONSOLE_DISPLAY_STATE};
use windows::Win32::Devices::Usb::GUID_DEVINTERFACE_USB_DEVICE;
use windows::Win32::System::WindowsProgramming::GetUserNameW;
use windows::core::PWSTR;

use crate::tts_engine::VoiceDetail;
use crate::config::Config;
use crate::event_monitor::{start_monitoring, SystemEvent, ConnectionType, IS_SYSTEM_ASLEEP};
use crate::i18n::I18nManager;
use crate::tts_engine::TtsEngine;

const WM_APP_TRAY_MSG: u32 = WM_APP + 1;
const WM_APP_WAKEUP: u32 = WM_APP + 2;
const ID_MENU_PAUSE_RESUME: u32 = 1001;
const ID_MENU_SETTINGS: u32 = 1002;
const ID_MENU_EXIT: u32 = 1003;

struct WindowProcData {
    sender: mpsc::Sender<SystemEvent>,
    app_state: Arc<Mutex<AppState>>,
}

struct AppState {
    is_paused: bool,
    tts_engine: TtsEngine,
    i18n_manager: I18nManager,
    username: String,
    last_usb_connect_time: Option<Instant>,
    last_usb_disconnect_time: Option<Instant>,
    config: Config,
    available_voices: Vec<VoiceDetail>,
}

fn set_working_directory() -> Result<(), Box<dyn Error>> {
    let current_exe = env::current_exe()?;
    if let Some(parent_dir) = current_exe.parent() {
        env::set_current_dir(parent_dir)?;
    } else {
        return Err("无法获取可执行文件的父目录".into());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    if let Err(e) = set_working_directory() {
        return Err(format!("设置工作目录失败: {}", e).into());
    }
    
    simple_logging::log_to_file("advanced_beeper.log", log::LevelFilter::Info)?;
    info!("-----------------------------------------");
    info!("高级提示 (Advanced Beeper) 应用程式启动");
    info!("-----------------------------------------");
    info!("工作目录已设置为可执行文件所在目录。");

    // --- CORE FIX: 为主线程初始化 COM ---
    // 这对于所有使用 WinRT 的操作（如此处的 TTS）都是必需的。
    let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if result.is_err() {
        // 将失败的 HRESULT 转换为可读的 Error 类型以便记录日志
        let error = windows::core::Error::from(result);
        error!("主线程 COM 初始化失败: {}", error);
        return Err(Box::new(error));
    }
    info!("主线程 COM (STA) 初始化成功。");


    let config = Config::load();
    info!("配置文件 config.json 已加载: {:?}", config);

    let effective_locale = match &config.language {
        Some(lang_code) => lang_code.clone(),
        None => {
            match sys_locale::get_locale() {
                Some(sys_lang) => {
                    let lang_prefix = sys_lang.split('-').next().unwrap_or(&sys_lang);
                    match lang_prefix {
                        "zh" => "zh".to_string(),
                        "ja" => "ja".to_string(),
                        _ => "en".to_string()
                    }
                },
                None => "en".to_string()
            }
        }
    };

    if let Err(e) = startup::set_auto_start(config.auto_start) {
        error!("启动时同步开机自启动设置失败: {}", e);
    }

    let (sender, receiver) = mpsc::channel();
    
    let tts_engine = {
        let mut engine = None;
        for attempt in 1..=3 {
            match TtsEngine::new(&config) {
                Ok(e) => {
                    info!("TTS 语音引擎在第 {} 次尝试时初始化成功。", attempt);
                    engine = Some(e);
                    break;
                },
                Err(e) => {
                    warn!("TTS 语音引擎初始化失败 (尝试 {}/3): {}", attempt, e);
                    if attempt < 3 {
                        std::thread::sleep(Duration::from_secs(3));
                    }
                }
            }
        }
        engine.ok_or_else(|| Box::<dyn Error>::from("TTS 引擎在3次尝试后仍无法初始化"))?
    };

    // 为这个调用增加日志，以便调试
    let available_voices = match tts_engine.list_available_voices() {
        Ok(voices) => {
            info!("成功获取到 {} 个可用语音。", voices.len());
            voices
        },
        Err(e) => {
            error!("获取可用语音列表失败: {}", e);
            // 即使失败，也继续运行，只是没有语音列表
            vec![] 
        }
    };

    let i18n_manager = I18nManager::new(&effective_locale)?;
    info!("国际化语言档案 (locale: {}) 载入成功。", effective_locale);

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

    if let Err(e) = sender.send(SystemEvent::SystemStartup) {
        error!("在启动时发送 SystemStartup 事件失败: {}", e);
    }

    let window_proc_data = Box::into_raw(Box::new(WindowProcData {
        sender: sender.clone(),
        app_state: app_state.clone(),
    }));
    
    let class_name = w!("AdvancedPromptsHiddenWindowClass");
    let instance = unsafe { GetModuleHandleW(None)? };
    let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class_name, ..Default::default() };
    
    let atom = unsafe { RegisterClassW(&wc) };
    if atom == 0 { return Err(Box::new(windows::core::Error::from_win32())); }

    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(), class_name, w!("CO/MP/UT/ER"), WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
            None, None, Some(instance.into()), Some(window_proc_data as *mut c_void),
        )?
    };

    start_monitoring(sender, hwnd);
    info!("已分派背景事件监控线程。");

    let mut msg = MSG::default();
    loop {
        while let Ok(event) = receiver.try_recv() {
            handle_system_event(event, &app_state);
        }

        let result = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !result.as_bool() { break; }

        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    
    Ok(())
}

// ... wndproc 和其他函数保持不变 ...
extern "system" fn wndproc(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if message == WM_CREATE {
        let create_struct = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        let data_ptr = create_struct.lpCreateParams as *mut WindowProcData;
        unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, data_ptr as isize); }
        add_tray_icon(window);
        
        if unsafe { RegisterPowerSettingNotification(window.into(), &GUID_ACDC_POWER_SOURCE, REGISTER_NOTIFICATION_FLAGS(0)) }.is_err() {
            error!("注册 AC/DC 电源通知失败。");
        }
        if unsafe { RegisterPowerSettingNotification(window.into(), &GUID_CONSOLE_DISPLAY_STATE, REGISTER_NOTIFICATION_FLAGS(0)) }.is_err() {
            error!("注册显示器状态通知失败。");
        }
        
        let mut filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_classguid: GUID_DEVINTERFACE_USB_DEVICE,
            ..Default::default()
        };
        if unsafe { RegisterDeviceNotificationW(window.into(), &mut filter as *mut _ as *mut c_void, DEVICE_NOTIFY_WINDOW_HANDLE) }.is_err() {
            error!("注册 USB 设备插拔通知失败。");
        }

        return LRESULT(0);
    }

    let data_ptr = unsafe { GetWindowLongPtrW(window, GWLP_USERDATA) } as *mut WindowProcData;
    if data_ptr.is_null() { return unsafe { DefWindowProcW(window, message, wparam, lparam) }; }
    
    let data = unsafe { &*data_ptr };
    let sender = &data.sender;
    let app_state_arc = &data.app_state;
    
    match message {
        WM_DEVICECHANGE => {
            let event = match wparam.0 as u32 {
                DBT_DEVICEARRIVAL => Some(SystemEvent::UsbDeviceConnected),
                DBT_DEVICEREMOVECOMPLETE => Some(SystemEvent::UsbDeviceDisconnected),
                _ => None
            };
            if let Some(event) = event {
                if lparam.0 != 0 {
                    let hdr = unsafe { &*(lparam.0 as *const DEV_BROADCAST_HDR) };
                    if hdr.dbch_devicetype == DBT_DEVTYP_DEVICEINTERFACE {
                        handle_debounced_usb_event(event, sender, app_state_arc, window);
                    }
                }
            }
            LRESULT(0)
        }
        
        WM_POWERBROADCAST => {
            match wparam.0 as u32 {
                PBT_APMSUSPEND => {
                    *IS_SYSTEM_ASLEEP.lock().unwrap() = true;
                    if sender.send(SystemEvent::SystemGoingToSleep).is_ok() {
                        unsafe { PostMessageW(Some(window), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
                    }
                }
                PBT_APMRESUMEAUTOMATIC => {
                    *IS_SYSTEM_ASLEEP.lock().unwrap() = false;
                    if sender.send(SystemEvent::SystemResumedFromSleep).is_ok() {
                        unsafe { PostMessageW(Some(window), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
                    }
                }
                PBT_POWERSETTINGCHANGE => {
                    let pbs = unsafe { &*(lparam.0 as *const POWERBROADCAST_SETTING) };
                    if pbs.PowerSetting == GUID_ACDC_POWER_SOURCE {
                        if !*IS_SYSTEM_ASLEEP.lock().unwrap() {
                            let source = unsafe { *(pbs.Data.as_ptr() as *const u32) };
                            let event = if source == 0 { SystemEvent::PowerSwitchedToAC } else { SystemEvent::PowerSwitchedToBattery };
                            if sender.send(event).is_ok() {
                                unsafe { PostMessageW(Some(window), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
                            }
                        }
                    } 
                    else if pbs.PowerSetting == GUID_CONSOLE_DISPLAY_STATE {
                        let display_state = unsafe { *(pbs.Data.as_ptr() as *const u32) };
                        let mut is_asleep_guard = IS_SYSTEM_ASLEEP.lock().unwrap();
                        match display_state {
                            0 if !*is_asleep_guard => {
                                *is_asleep_guard = true;
                                drop(is_asleep_guard);
                                if sender.send(SystemEvent::SystemGoingToSleep).is_ok() {
                                    unsafe { PostMessageW(Some(window), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
                                }
                            },
                            1 if *is_asleep_guard => {
                                *is_asleep_guard = false;
                                drop(is_asleep_guard);
                                if sender.send(SystemEvent::SystemResumedFromSleep).is_ok() {
                                    unsafe { PostMessageW(Some(window), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
                                }
                            },
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }

        WM_APP_WAKEUP => LRESULT(0),

        WM_APP_TRAY_MSG => {
            if (lparam.0 as u32 & 0xFFFF) == WM_RBUTTONUP {
                let menu = unsafe { CreatePopupMenu().unwrap() };
                let app_state = app_state_arc.lock().unwrap();
                let i18n = &app_state.i18n_manager;
                let pause_resume_text_key = if app_state.is_paused { "menu_resume" } else { "menu_pause" };
                let pause_resume_text = i18n.get_text(pause_resume_text_key).unwrap_or_else(|| "Pause/Resume".to_string());
                let settings_text = i18n.get_text("menu_settings").unwrap_or_else(|| "Settings...".to_string());
                let exit_text = i18n.get_text("menu_exit").unwrap_or_else(|| "Exit".to_string());
                unsafe {
                    AppendMenuW(menu, MF_STRING, ID_MENU_PAUSE_RESUME as usize, &HSTRING::from(pause_resume_text)).ok();
                    AppendMenuW(menu, MF_STRING, ID_MENU_SETTINGS as usize, &HSTRING::from(settings_text)).ok();
                    AppendMenuW(menu, MF_STRING, ID_MENU_EXIT as usize, &HSTRING::from(exit_text)).ok();
                    let mut point = Default::default();
                    GetCursorPos(&mut point).ok();
                    SetForegroundWindow(window);
                    TrackPopupMenu(menu, TPM_BOTTOMALIGN | TPM_LEFTALIGN, point.x, point.y, Some(0), window, None).ok();
                }
            }
            LRESULT(0)
        }

        WM_COMMAND => {
            match wparam.0 as u32 {
                ID_MENU_PAUSE_RESUME => {
                    let mut app_state = app_state_arc.lock().unwrap();
                    app_state.is_paused = !app_state.is_paused;
                    let announcement_key = if app_state.is_paused { "announcement_paused" } else { "announcement_resumed" };
                    if let Some(text) = app_state.i18n_manager.get_text(announcement_key) {
                        app_state.tts_engine.speak(&text).ok();
                    }
                }
                ID_MENU_SETTINGS => settings_ui::show(window, app_state_arc.clone()),
                ID_MENU_EXIT => {
                    {
                        let mut app_state = app_state_arc.lock().unwrap();
                        if let Some(text) = app_state.i18n_manager.get_text("announcement_exit") {
                           app_state.tts_engine.speak(&text).ok();
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    unsafe { DestroyWindow(window) };
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            remove_tray_icon(window);
            let _ = unsafe { Box::from_raw(SetWindowLongPtrW(window, GWLP_USERDATA, 0) as *mut WindowProcData) };
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

fn handle_system_event(event: SystemEvent, app_state_arc: &Arc<Mutex<AppState>>) {
    if *IS_SYSTEM_ASLEEP.lock().unwrap() && !matches!(event, SystemEvent::SystemResumedFromSleep) { return; }
    if matches!(event, SystemEvent::SystemGoingToSleep) { return; }
    let mut app_state = app_state_arc.lock().unwrap();
    if app_state.is_paused { return; }
    
    let i18n = &app_state.i18n_manager;
    let text_to_speak = match &event {
        SystemEvent::SystemStartup => i18n.get_text_with_param("system_online", "user", &app_state.username),
        SystemEvent::PowerSwitchedToAC => i18n.get_text("external_power_connected"),
        SystemEvent::PowerSwitchedToBattery => i18n.get_text("switched_to_battery"),
        SystemEvent::BatteryLevelReport(level) => i18n.get_text_with_param("battery_level_report", "level", &level.to_string()),
        SystemEvent::UsbDeviceConnected => i18n.get_text("usb_device_detected"),
        SystemEvent::UsbDeviceDisconnected => i18n.get_text("usb_device_disconnected"),
        SystemEvent::BatteryInserted => i18n.get_text("battery_inserted"),
        SystemEvent::BatteryRemoved => i18n.get_text("battery_removed"),
        SystemEvent::NetworkConnected { name, conn_type } => match conn_type {
            ConnectionType::WiFi => i18n.get_text_with_param("network_connected_wifi", "SSID", name),
            _ => i18n.get_text("network_connected_ethernet"),
        },
        SystemEvent::NetworkDisconnected => i18n.get_text("network_disconnected"),
        SystemEvent::SystemResumedFromSleep => i18n.get_text("system_resumed_from_sleep"),
        _ => None, 
    };
    
    if let Some(text) = text_to_speak {
        app_state.tts_engine.speak(&text).ok();
    }
}

const USB_DEBOUNCE_DURATION: Duration = Duration::from_secs(2);

fn handle_debounced_usb_event(
    event: SystemEvent, 
    sender: &mpsc::Sender<SystemEvent>, 
    app_state_arc: &Arc<Mutex<AppState>>,
    window: HWND,
) {
    let mut app_state = app_state_arc.lock().unwrap();
    let now = Instant::now();
    let should_send = match event {
        SystemEvent::UsbDeviceConnected => {
            let last_time = app_state.last_usb_connect_time.get_or_insert(now);
            if now.duration_since(*last_time) < USB_DEBOUNCE_DURATION && *last_time != now { false }
            else { *last_time = now; true }
        }
        SystemEvent::UsbDeviceDisconnected => {
            let last_time = app_state.last_usb_disconnect_time.get_or_insert(now);
            if now.duration_since(*last_time) < USB_DEBOUNCE_DURATION && *last_time != now { false }
            else { *last_time = now; true }
        }
        _ => true,
    };

    if should_send {
        if sender.send(event).is_ok() {
            unsafe { PostMessageW(Some(window), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
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
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        nid.hIcon = LoadIconW(Some(instance.into()), PCWSTR(1 as *const u16)).unwrap_or_else(|_| LoadIconW(None, IDI_APPLICATION).unwrap());
    }
    let tip = w!("CO/MP/UT/ER");
    let tip_wide = unsafe { tip.as_wide() };
    nid.szTip[..tip_wide.len()].copy_from_slice(tip_wide);
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
}

fn remove_tray_icon(hwnd: HWND) {
    let mut nid = NOTIFYICONDATAW::default();
    nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
}