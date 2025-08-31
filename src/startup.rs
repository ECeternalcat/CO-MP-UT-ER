// src/startup.rs

use std::env;
use winreg::enums::*;
use winreg::RegKey;
use log::info;

const APP_NAME: &str = "co_mp_ut_er";
const REG_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// 根据传入的布尔值，在 Windows 注册表中添加或移除本应用的开机自启动项。
pub fn set_auto_start(enable: bool) -> Result<(), std::io::Error> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu.open_subkey_with_flags(REG_KEY_PATH, KEY_WRITE)?;

    if enable {
        let exe_path = env::current_exe()?;
        let exe_path_str = exe_path.to_str().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Other, "可执行文件路径包含无效的 UTF-8 字符")
        })?;
        // 为路径添加引号，以防路径中包含空格
        let value = format!("\"{}\"", exe_path_str);
        run_key.set_value(APP_NAME, &value)?;
        info!("已设置开机自启动。路径: {}", value);
    } else {
        // 如果值不存在，delete_value 会返回错误，这是正常情况，我们忽略它。
        if run_key.delete_value(APP_NAME).is_ok() {
            info!("已取消开机自启动。");
        }
    }

    Ok(())
}