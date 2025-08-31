// src/settings_ui.rs

use std::sync::{Arc, Mutex};
use std::ffi::c_void;
use once_cell::sync::Lazy;

// --- 核心修复：从正确的模块引入所有需要的函数 ---
use windows::core::{w, HSTRING, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, WHITE_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::SS_LEFT;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, GetWindowLongPtrW, LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetWindowLongPtrW, TranslateMessage, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, CBS_DROPDOWNLIST, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, HMENU, IDC_ARROW, MSG, WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_EX_DLGMODALFRAME, WS_SYSMENU, WS_VISIBLE, WS_VSCROLL};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow};


use crate::i18n::I18nManager;
use crate::AppState;
use log::{error, info, warn};

const IDC_VOICE_LABEL: i32 = 101;
const IDC_VOICE_COMBO: i32 = 102;
const IDC_AUTOSTART_CHECK: i32 = 103;
const IDC_LANG_LABEL: i32 = 104;
const IDC_LANG_COMBO: i32 = 105;
const IDOK: i32 = 1;
const IDCANCEL: i32 = 2;

static SETTINGS_CLASS_NAME: Lazy<HSTRING> = Lazy::new(|| HSTRING::from("AdvancedBeeperSettingsWindowClass"));

struct SettingsWindowData {
    app_state: Arc<Mutex<AppState>>,
    h_voice_combo: HWND,
    h_autostart_check: HWND,
    h_lang_combo: HWND,
}

fn register_settings_class() {
    static REGISTER_ONCE: std::sync::Once = std::sync::Once::new();
    REGISTER_ONCE.call_once(|| {
        let instance = unsafe { GetModuleHandleW(None).unwrap() };

        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(settings_wnd_proc),
            hInstance: instance.into(),
            hCursor: unsafe { LoadCursorW(None, IDC_ARROW).unwrap_or_default() },
            hbrBackground: HBRUSH(unsafe { GetStockObject(WHITE_BRUSH) }.0),
            lpszClassName: PCWSTR((&*SETTINGS_CLASS_NAME).as_ptr()),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&wc) } == 0 {
            error!("注册设置窗口类失败: {}", windows::core::Error::from_win32());
        }
    });
}

pub fn show(parent: HWND, app_state: Arc<Mutex<AppState>>) {
    register_settings_class();
    let instance = unsafe { GetModuleHandleW(None).unwrap() };

    // --- 新增: 在创建窗口前，先获取翻译后的标题 ---
    let window_title = {
        // 使用一个代码块来限制锁的生命周期
        let state = app_state.lock().unwrap();
        state.i18n_manager.get_text("settings_window_title").unwrap_or_else(|| "Settings".to_string())
    };

    let data = Box::new(SettingsWindowData {
        app_state,
        h_voice_combo: HWND::default(),
        h_autostart_check: HWND::default(),
        h_lang_combo: HWND::default(),
    });

    let data_ptr = Box::into_raw(data);

    let hwnd_result: Result<HWND> = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            &*SETTINGS_CLASS_NAME,
            // --- 修改: 使用我们刚刚获取的翻译后的标题 ---
            &HSTRING::from(window_title),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT, CW_USEDEFAULT, 400, 220, // 调整了窗口高度以适应新布局
            Some(parent),
            None,
            Some(instance.into()),
            Some(data_ptr as *mut c_void),
        )
    };
    
    // 现在 EnableWindow 已经被正确引入，可以被找到了
    unsafe { let _ = EnableWindow(parent, false); };
    
    let mut msg = MSG::default();
    
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    
    unsafe { 
        let _ = EnableWindow(parent, true);
        SetActiveWindow(parent).ok();
    }
}

extern "system" fn settings_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_CREATE => {
            let create_struct = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            let data_ptr = create_struct.lpCreateParams as *mut SettingsWindowData;
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize) };

            let data = unsafe { &mut *data_ptr };
            create_controls(hwnd, data);
            initialize_controls(data);
            LRESULT(0)
        }
        windows::Win32::UI::WindowsAndMessaging::WM_COMMAND => {
            let id = (wparam.0 as u16) as i32;
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindowData };
            if data_ptr.is_null() { return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }; }
            let data = unsafe { &mut *data_ptr };

            match id {
                IDOK => {
                    save_settings(data);
                    // FIX: 将 hwnd 用 Some() 包裹
                    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).ok() };
                }
                IDCANCEL => {
                    // FIX: 将 hwnd 用 Some() 包裹
                    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).ok() };
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            unsafe { DestroyWindow(hwnd).ok() };
            LRESULT(0)
        }
        WM_DESTROY => {
            let data_ptr = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut SettingsWindowData };
            if !data_ptr.is_null() {
                let _ = unsafe { Box::from_raw(data_ptr) };
            }
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn create_controls(parent: HWND, data: &mut SettingsWindowData) {
    let instance = unsafe { GetModuleHandleW(None).unwrap() };
    
    // --- 新增: 在函数开头一次性获取所有需要的翻译文本 ---
    // 这样做是为了保持代码整洁，并尽量缩短互斥锁的锁定时间。
    let (lbl_voice, lbl_lang, chk_autostart, btn_ok, btn_cancel) = {
        let app_state = data.app_state.lock().unwrap();
        let i18n = &app_state.i18n_manager;
        (
            i18n.get_text("settings_label_voice").unwrap_or_else(|| "Voice:".to_string()),
            i18n.get_text("settings_label_language").unwrap_or_else(|| "Language:".to_string()),
            i18n.get_text("settings_checkbox_autostart").unwrap_or_else(|| "Start with Windows".to_string()),
            i18n.get_text("settings_button_ok").unwrap_or_else(|| "OK".to_string()),
            i18n.get_text("settings_button_cancel").unwrap_or_else(|| "Cancel".to_string()),
        )
    };

    unsafe {
        // --- 语音选择 (Voice) ---
        // 修改: 使用翻译后的文本 &HSTRING::from(lbl_voice)
        CreateWindowExW(Default::default(), w!("STATIC"), &HSTRING::from(lbl_voice), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT.0), 20, 20, 80, 25, Some(parent), Some(HMENU((IDC_VOICE_LABEL as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        data.h_voice_combo = CreateWindowExW(Default::default(), w!("COMBOBOX"), None, WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | (CBS_DROPDOWNLIST as u32) | WS_VSCROLL.0), 100, 20, 250, 200, Some(parent), Some(HMENU((IDC_VOICE_COMBO as isize) as *mut c_void)), Some(instance.into()), None).unwrap();

        // --- 语言选择 (Language) ---
        // 修改: 使用翻译后的文本 &HSTRING::from(lbl_lang)
        CreateWindowExW(Default::default(), w!("STATIC"), &HSTRING::from(lbl_lang), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT.0), 20, 70, 80, 25, Some(parent), Some(HMENU((IDC_LANG_LABEL as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        data.h_lang_combo = CreateWindowExW(Default::default(), w!("COMBOBOX"), None, WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | (CBS_DROPDOWNLIST as u32)), 100, 70, 250, 100, Some(parent), Some(HMENU((IDC_LANG_COMBO as isize) as *mut c_void)), Some(instance.into()), None).unwrap();

        // --- 开机自启动 (Start with Windows) ---
        // 修改: 使用翻译后的文本 &HSTRING::from(chk_autostart)
        data.h_autostart_check = CreateWindowExW(Default::default(), w!("BUTTON"), &HSTRING::from(chk_autostart), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | (BS_AUTOCHECKBOX as u32)), 20, 110, 200, 25, Some(parent), Some(HMENU((IDC_AUTOSTART_CHECK as isize) as *mut c_void)), Some(instance.into()), None).unwrap();

        // --- 按钮 ---
        // 修改: 使用翻译后的文本 &HSTRING::from(btn_ok) 和 &HSTRING::from(btn_cancel)
        CreateWindowExW(Default::default(), w!("BUTTON"), &HSTRING::from(btn_ok), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | (BS_DEFPUSHBUTTON as u32)), 120, 150, 100, 30, Some(parent), Some(HMENU((IDOK as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        CreateWindowExW(Default::default(), w!("BUTTON"), &HSTRING::from(btn_cancel), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0), 240, 150, 100, 30, Some(parent), Some(HMENU((IDCANCEL as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
    }
}

fn initialize_controls(data: &mut SettingsWindowData) {
    let app_state = data.app_state.lock().unwrap();
    let config = &app_state.config;

    // --- 初始化语言下拉框 ---
    let supported_langs = vec![("en", "English"), ("zh", "简体中文"), ("ja", "日本語")];
    let mut lang_selected_index = 0;
    for (i, (code, display_name)) in supported_langs.iter().enumerate() {
        let h_name = HSTRING::from(*display_name);
        unsafe { SendMessageW(data.h_lang_combo, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(h_name.as_ptr() as isize))); }
        if config.language.as_deref() == Some(*code) {
            lang_selected_index = i;
        }
    }
    unsafe { SendMessageW(data.h_lang_combo, CB_SETCURSEL, Some(WPARAM(lang_selected_index)), Some(LPARAM(0))); }

    // --- 初始化自启动复选框 ---
    unsafe { 
        SendMessageW(
            data.h_autostart_check, 
            BM_SETCHECK, 
            Some(WPARAM(if config.auto_start { BST_CHECKED.0 as usize } else { BST_UNCHECKED.0 as usize })), 
            Some(LPARAM(0))
        ); 
    }

    // --- 初始化语音下拉框 (唯一的一次) ---
    let voices = &app_state.available_voices;
    if voices.is_empty() {
        let unavailable_msg = HSTRING::from("<Unavailable>");
        unsafe { SendMessageW(data.h_voice_combo, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(unavailable_msg.as_ptr() as isize))); }
    } else {
        let mut selected_index: usize = 0;
        for (i, voice) in voices.iter().enumerate() {
            // 组合显示文本
            let display_text = format!("{} ({})", voice.name, voice.language);
            let h_display_text = HSTRING::from(display_text.as_str());
            
            unsafe { SendMessageW(data.h_voice_combo, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(h_display_text.as_ptr() as isize))); }
            
            // 匹配纯名称
            if config.custom_voice.as_deref() == Some(&voice.name) {
                selected_index = i;
            }
        }
        unsafe { SendMessageW(data.h_voice_combo, CB_SETCURSEL, Some(WPARAM(selected_index)), Some(LPARAM(0))); }
    }
}

fn save_settings(data: &mut SettingsWindowData) {
    let mut app_state = data.app_state.lock().unwrap();


    // --- 保存语言设置 ---
    let lang_index = unsafe { SendMessageW(data.h_lang_combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as i32;
    let lang_codes = ["en", "zh", "ja"];
    if lang_index >= 0 && (lang_index as usize) < lang_codes.len() {
        let selected_lang_code = lang_codes[lang_index as usize];
        
        // --- 核心修改 ---
        // 检查用户是否真的更改了语言
        if app_state.config.language.as_deref() != Some(selected_lang_code) {
            info!("语言已从 {:?} 更改为 '{}'", app_state.config.language, selected_lang_code);
            
            // 1. 更新语言配置
            app_state.config.language = Some(selected_lang_code.to_string());
            
            // 2. 清除已选择的语音，以便下次启动时能自动匹配新语言
            info!("由于语言已更改，将清除自定义语音设置以进行重新匹配。");
            app_state.config.custom_voice = None;

            // 3. 立即应用新的语言设置
            match I18nManager::new(selected_lang_code) {
                Ok(new_i18n_manager) => {
                    app_state.i18n_manager = new_i18n_manager;
                    info!("语言已动态切换为 '{}'", selected_lang_code);
                },
                Err(e) => error!("动态切换语言失败: {}", e),
            }
        }
    }

    // FIX: 将 WPARAM 和 LPARAM 参数用 Some() 包裹
    let is_checked = unsafe { SendMessageW(data.h_autostart_check, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as u32 == BST_CHECKED.0;
    app_state.config.auto_start = is_checked;
    
    if let Err(e) = crate::startup::set_auto_start(is_checked) {
        error!("保存开机自启动设置到注册表失败: {}", e);
    }

    let selected_index = unsafe { SendMessageW(data.h_voice_combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as i32;
    if selected_index >= 0 {
        // 直接从缓存的详细列表中根据索引获取语音名称
        if let Some(selected_voice) = app_state.available_voices.get(selected_index as usize) {
            let voice_name_to_save = selected_voice.name.clone();
            info!("设置窗口: 选中的语音是 '{}'", voice_name_to_save);

            // 更新配置对象
            app_state.config.custom_voice = Some(voice_name_to_save.clone());

            // 立即应用新的语音
            if let Err(e) = app_state.tts_engine.set_voice(&voice_name_to_save) {
                error!("动态应用新语音失败: {}", e);
            }
        } else {
            warn!("未能根据索引 {} 找到对应的语音信息。", selected_index);
        }
    }

    if let Err(e) = app_state.config.save() {
        error!("保存 config.json 文件失败: {}", e);
    }
}