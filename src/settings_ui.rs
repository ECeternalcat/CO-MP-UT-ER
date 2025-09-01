// src/settings_ui.rs

use std::sync::{Arc, Mutex};
use std::ffi::c_void;
use once_cell::sync::Lazy;

// --- 核心修复：引入新版API所需的具体枚举和类型 ---
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
// --- 修改: 引入CreateFontW所需的强类型枚举常量 ---
use windows::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetStockObject, HBRUSH, HFONT, WHITE_BRUSH,
    DEFAULT_GUI_FONT, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, DEFAULT_QUALITY, FF_DONTCARE,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemServices::SS_LEFT;
use windows::Win32::UI::Controls::{BST_CHECKED, BST_UNCHECKED};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, GetWindowLongPtrW,
    LoadCursorW, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetWindowLongPtrW,
    TranslateMessage, BM_GETCHECK, BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, CBS_DROPDOWNLIST,
    CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    GWLP_USERDATA, HMENU, IDC_ARROW, MSG, WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY,
    WM_SETFONT, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_EX_DLGMODALFRAME, WS_SYSMENU, WS_VISIBLE, WS_VSCROLL,
};
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
    h_font: HFONT,
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

    let window_title = {
        let state = app_state.lock().unwrap();
        state.i18n_manager.get_text("settings_window_title").unwrap_or_else(|| "Settings".to_string())
    };

    let data = Box::new(SettingsWindowData {
        app_state,
        h_voice_combo: HWND::default(),
        h_autostart_check: HWND::default(),
        h_lang_combo: HWND::default(),
        h_font: HFONT::default(),
    });

    let data_ptr = Box::into_raw(data);

    // 使用 match 或者 ? 来处理 Result
    if let Err(e) = unsafe {
        CreateWindowExW(
            WS_EX_DLGMODALFRAME,
            &*SETTINGS_CLASS_NAME,
            &HSTRING::from(window_title),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            CW_USEDEFAULT, CW_USEDEFAULT, 400, 220,
            Some(parent),
            None,
            Some(instance.into()),
            Some(data_ptr as *mut c_void),
        )
    } {
        error!("创建设置窗口失败: {}", e);
        // 如果窗口创建失败，需要释放 data_ptr 以避免内存泄漏
        unsafe { let _ = Box::from_raw(data_ptr); };
        return;
    }
    
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
        WM_CREATE => {
            let create_struct = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            let data_ptr = create_struct.lpCreateParams as *mut SettingsWindowData;
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, data_ptr as isize) };

            let data = unsafe { &mut *data_ptr };

            let font_name = w!("Microsoft YaHei UI");
            data.h_font = unsafe {
                CreateFontW(
                    -15,                // nHeight
                    0,                  // nWidth
                    0,                  // nEscapement
                    0,                  // nOrientation
                    400,                // --- 核心修复: 直接使用整数 400 替代 FW_NORMAL.0 ---
                    0,                  // fdwItalic
                    0,                  // fdwUnderline
                    0,                  // fdwStrikeOut
                    DEFAULT_CHARSET,    // fdwCharSet
                    OUT_DEFAULT_PRECIS, // fdwOutputPrecision
                    CLIP_DEFAULT_PRECIS,// fdwClipPrecision
                    DEFAULT_QUALITY,    // fdwQuality
                    FF_DONTCARE.0.into(),   // fdwPitchAndFamily
                    font_name,          // pszFaceName
                )
            };

            if data.h_font.is_invalid() {
                warn!("创建 'Microsoft YaHei UI' 字体失败, 回退到系统默认字体。");
                data.h_font = HFONT(unsafe { GetStockObject(DEFAULT_GUI_FONT) }.0);
            }

            create_controls(hwnd, data);
            initialize_controls(data);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 as u16) as i32;
            let data_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsWindowData };
            if data_ptr.is_null() { return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }; }
            let data = unsafe { &mut *data_ptr };

            match id {
                IDOK => {
                    save_settings(data);
                    unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)).ok() };
                }
                IDCANCEL => {
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
                let data = unsafe { Box::from_raw(data_ptr) };
                
                let default_font = HFONT(unsafe { GetStockObject(DEFAULT_GUI_FONT) }.0);
                if !data.h_font.is_invalid() && data.h_font != default_font {
                    unsafe { let _ = DeleteObject(data.h_font.into()); };
                }
            }
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn create_controls(parent: HWND, data: &mut SettingsWindowData) {
    let instance = unsafe { GetModuleHandleW(None).unwrap() };
    let h_font = data.h_font;
    
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
        let set_font = |hwnd: HWND| {
            if !h_font.is_invalid() {
                // --- 修复: 将 WPARAM 和 LPARAM 用 Some() 包裹 ---
                SendMessageW(hwnd, WM_SETFONT, Some(WPARAM(h_font.0 as usize)), Some(LPARAM(1)));
            }
        };

        // --- 语音选择 (Voice) ---
        let h_voice_label = CreateWindowExW(Default::default(), w!("STATIC"), &HSTRING::from(lbl_voice), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT.0), 20, 20, 80, 25, Some(parent), Some(HMENU((IDC_VOICE_LABEL as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        set_font(h_voice_label);
        
        data.h_voice_combo = CreateWindowExW(Default::default(), w!("COMBOBOX"), None, WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | (CBS_DROPDOWNLIST as u32) | WS_VSCROLL.0), 100, 20, 250, 200, Some(parent), Some(HMENU((IDC_VOICE_COMBO as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        set_font(data.h_voice_combo);

        // --- 语言选择 (Language) ---
        let h_lang_label = CreateWindowExW(Default::default(), w!("STATIC"), &HSTRING::from(lbl_lang), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | SS_LEFT.0), 20, 70, 80, 25, Some(parent), Some(HMENU((IDC_LANG_LABEL as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        set_font(h_lang_label);

        data.h_lang_combo = CreateWindowExW(Default::default(), w!("COMBOBOX"), None, WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | (CBS_DROPDOWNLIST as u32)), 100, 70, 250, 100, Some(parent), Some(HMENU((IDC_LANG_COMBO as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        set_font(data.h_lang_combo);

        // --- 开机自启动 (Start with Windows) ---
        data.h_autostart_check = CreateWindowExW(Default::default(), w!("BUTTON"), &HSTRING::from(chk_autostart), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | (BS_AUTOCHECKBOX as u32)), 20, 110, 200, 25, Some(parent), Some(HMENU((IDC_AUTOSTART_CHECK as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        set_font(data.h_autostart_check);

        // --- 按钮 ---
        let h_ok_btn = CreateWindowExW(Default::default(), w!("BUTTON"), &HSTRING::from(btn_ok), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | (BS_DEFPUSHBUTTON as u32)), 120, 150, 100, 30, Some(parent), Some(HMENU((IDOK as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        set_font(h_ok_btn);
        
        let h_cancel_btn = CreateWindowExW(Default::default(), w!("BUTTON"), &HSTRING::from(btn_cancel), WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0), 240, 150, 100, 30, Some(parent), Some(HMENU((IDCANCEL as isize) as *mut c_void)), Some(instance.into()), None).unwrap();
        set_font(h_cancel_btn);
    }
}

fn initialize_controls(data: &mut SettingsWindowData) {
    let app_state = data.app_state.lock().unwrap();
    let config = &app_state.config;

    let supported_langs = vec![("en", "English"), ("zh", "简体中文"), ("ja", "日本語")];
    let mut lang_selected_index = 0;
    for (i, (code, display_name)) in supported_langs.iter().enumerate() {
        let h_name = HSTRING::from(*display_name);
        // --- 修复: 将 LPARAM 用 Some() 包裹 ---
        unsafe { SendMessageW(data.h_lang_combo, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(h_name.as_ptr() as isize))); }
        if config.language.as_deref() == Some(*code) {
            lang_selected_index = i;
        }
    }
    // --- 修复: 将 WPARAM 和 LPARAM 用 Some() 包裹 ---
    unsafe { SendMessageW(data.h_lang_combo, CB_SETCURSEL, Some(WPARAM(lang_selected_index)), Some(LPARAM(0))); }

    unsafe { 
        // --- 修复: 将 WPARAM 和 LPARAM 用 Some() 包裹 ---
        SendMessageW(
            data.h_autostart_check, 
            BM_SETCHECK, 
            Some(WPARAM(if config.auto_start { BST_CHECKED.0 as usize } else { BST_UNCHECKED.0 as usize })), 
            Some(LPARAM(0))
        ); 
    }

    let voices = &app_state.available_voices;
    if voices.is_empty() {
        let unavailable_msg = HSTRING::from("<Unavailable>");
        // --- 修复: 将 LPARAM 用 Some() 包裹 ---
        unsafe { SendMessageW(data.h_voice_combo, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(unavailable_msg.as_ptr() as isize))); }
    } else {
        let mut selected_index: usize = 0;
        for (i, voice) in voices.iter().enumerate() {
            let display_text = format!("{} ({})", voice.name, voice.language);
            let h_display_text = HSTRING::from(display_text.as_str());
            
            // --- 修复: 将 LPARAM 用 Some() 包裹 ---
            unsafe { SendMessageW(data.h_voice_combo, CB_ADDSTRING, Some(WPARAM(0)), Some(LPARAM(h_display_text.as_ptr() as isize))); }
            
            if config.custom_voice.as_deref() == Some(&voice.name) {
                selected_index = i;
            }
        }
        // --- 修复: 将 WPARAM 和 LPARAM 用 Some() 包裹 ---
        unsafe { SendMessageW(data.h_voice_combo, CB_SETCURSEL, Some(WPARAM(selected_index)), Some(LPARAM(0))); }
    }
}

fn save_settings(data: &mut SettingsWindowData) {
    let mut app_state = data.app_state.lock().unwrap();

    // --- 修复: 将 WPARAM 和 LPARAM 用 Some() 包裹 ---
    let lang_index = unsafe { SendMessageW(data.h_lang_combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as i32;
    let lang_codes = ["en", "zh", "ja"];
    if lang_index >= 0 && (lang_index as usize) < lang_codes.len() {
        let selected_lang_code = lang_codes[lang_index as usize];
        
        if app_state.config.language.as_deref() != Some(selected_lang_code) {
            info!("语言已从 {:?} 更改为 '{}'", app_state.config.language, selected_lang_code);
            
            app_state.config.language = Some(selected_lang_code.to_string());
            info!("由于语言已更改，将清除自定义语音设置以进行重新匹配。");
            app_state.config.custom_voice = None;

            match I18nManager::new(selected_lang_code) {
                Ok(new_i18n_manager) => {
                    app_state.i18n_manager = new_i18n_manager;
                    info!("语言已动态切换为 '{}'", selected_lang_code);
                },
                Err(e) => error!("动态切换语言失败: {}", e),
            }
        }
    }

    // --- 修复: 将 WPARAM 和 LPARAM 用 Some() 包裹 ---
    let is_checked = unsafe { SendMessageW(data.h_autostart_check, BM_GETCHECK, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as u32 == BST_CHECKED.0;
    app_state.config.auto_start = is_checked;
    
    if let Err(e) = crate::startup::set_auto_start(is_checked) {
        error!("保存开机自启动设置到注册表失败: {}", e);
    }
    // --- 修复: 将 WPARAM 和 LPARAM 用 Some() 包裹 ---
    let selected_index = unsafe { SendMessageW(data.h_voice_combo, CB_GETCURSEL, Some(WPARAM(0)), Some(LPARAM(0))) }.0 as i32;
    if selected_index >= 0 {
        if let Some(selected_voice) = app_state.available_voices.get(selected_index as usize) {
            let voice_name_to_save = selected_voice.name.clone();
            info!("设置窗口: 选中的语音是 '{}'", voice_name_to_save);

            app_state.config.custom_voice = Some(voice_name_to_save.clone());

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