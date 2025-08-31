// src/event_monitor.rs

use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use tokio::runtime::Runtime;

use log::{info, error, warn, debug};

use windows::core::IInspectable;
use windows::Foundation::{EventHandler, TypedEventHandler};
use windows::ApplicationModel::{Core::CoreApplication, SuspendingEventArgs};
use windows::Devices::Power::Battery;
use windows::Networking::Connectivity::{NetworkInformation, NetworkStatusChangedEventHandler};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

lazy_static::lazy_static! {
    pub static ref IS_SYSTEM_ASLEEP: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionType { Ethernet, WiFi, Cellular, Unknown }

#[derive(Debug)]
pub enum SystemEvent {
    PowerSwitchedToAC, PowerSwitchedToBattery, BatteryLevelReport(u8),
    UsbDeviceConnected, UsbDeviceDisconnected, SystemStartup,
    BatteryInserted, BatteryRemoved,
    NetworkConnected { name: String, conn_type: ConnectionType },
    NetworkDisconnected, SystemGoingToSleep, SystemResumedFromSleep,
}

pub fn start_monitoring(sender: mpsc::Sender<SystemEvent>) {
    thread::spawn(move || {
        info!("WinRT 监控线程已启动。");
        if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_err() {
            error!("WinRT 监控线程初始化 COM (STA) 失败，线程即将退出。");
            return;
        }
        
        match Runtime::new() {
            Ok(rt) => {
                info!("Tokio 运行时创建成功。");
                rt.block_on(async {
                    info!("进入 Tokio 运行时，准备启动所有监控任务。");
                    tokio::join!(
                        setup_lifecycle_monitor(sender.clone()),
                        setup_battery_monitor(sender.clone()),
                        setup_network_monitor(sender.clone())
                    );
                    info!("Tokio 运行时 block_on 结束。 (这不应该发生!)");
                });
            },
            Err(e) => {
                error!("创建 Tokio 运行时失败: {}", e);
            }
        }
    });
}

async fn setup_lifecycle_monitor(sender: mpsc::Sender<SystemEvent>) {
    info!("[Lifecycle Monitor] 任务启动。");
    let setup = || -> windows::core::Result<()> {
        let suspending_handler = EventHandler::<SuspendingEventArgs>::new({
            let sender_clone = sender.clone();
            move |_, _| {
                info!("WinRT: 接收到 Suspending 事件，系统即将进入睡眠。");
                *IS_SYSTEM_ASLEEP.lock().unwrap() = true;
                if let Err(e) = sender_clone.send(SystemEvent::SystemGoingToSleep) {
                    error!("发送 SystemGoingToSleep 事件失败: {}", e);
                }
                Ok(())
            }
        });

        let resuming_handler = EventHandler::<IInspectable>::new({
            let sender_clone = sender.clone();
            move |_, _| {
                info!("WinRT: 接收到 Resuming 事件，系统已从睡眠中唤醒。");
                *IS_SYSTEM_ASLEEP.lock().unwrap() = false;
                if let Err(e) = sender_clone.send(SystemEvent::SystemResumedFromSleep) {
                    error!("发送 SystemResumedFromSleep 事件失败: {}", e);
                }
                Ok(())
            }
        });

        CoreApplication::Suspending(&suspending_handler)?;
        info!("[Lifecycle Monitor] 成功订阅 WinRT Suspending 事件。");
        
        CoreApplication::Resuming(&resuming_handler)?;
        info!("[Lifecycle Monitor] 成功订阅 WinRT Resuming 事件。");
        
        Ok(())
    };

    if let Err(e) = setup() {
        error!("[Lifecycle Monitor] 订阅生命周期事件失败: {:?}", e);
    }
    
    std::future::pending::<()>().await;
}

async fn setup_battery_monitor(sender: mpsc::Sender<SystemEvent>) {
    info!("[Battery Monitor] 任务启动。");
    let aggregate_battery = match Battery::AggregateBattery() {
        Ok(b) => b,
        Err(_) => {
            info!("[Battery Monitor] 未检测到电池，监控器将不会启动。");
            return;
        }
    };

    let last_present_state = Arc::new(Mutex::new(None::<bool>));

    if let Ok(report) = aggregate_battery.GetReport() {
        let is_present = report.FullChargeCapacityInMilliwattHours().map_or(false, |cap| cap.GetInt32().map_or(false, |c| c > 0));
        *last_present_state.lock().unwrap() = Some(is_present);
    }

    let handler = TypedEventHandler::<Battery, IInspectable>::new({
        let sender_clone = sender.clone();
        let state_clone = last_present_state.clone();
        let battery_clone = aggregate_battery.clone(); 
        
        move |_, _| {
            if *IS_SYSTEM_ASLEEP.lock().unwrap() { return Ok(()); }
            info!("[Battery Monitor] ReportUpdated 事件触发。");
            
            let report_result = battery_clone.GetReport();
            let is_present = match report_result {
                Ok(report) => report.FullChargeCapacityInMilliwattHours().map_or(false, |cap| cap.GetInt32().map_or(false, |c| c > 0)),
                Err(_) => false,
            };
            
            let mut guard = state_clone.lock().unwrap();
            if let Some(was_present) = *guard {
                if was_present != is_present {
                    let event = if was_present && !is_present { SystemEvent::BatteryRemoved } else { SystemEvent::BatteryInserted };
                    info!("[Battery Monitor] 检测到电池插拔事件 -> {:?}", event);
                    if let Err(e) = sender_clone.send(event) {
                        error!("发送电池插拔事件失败: {}", e);
                    }
                }
            }
            *guard = Some(is_present);
            Ok(())
        }
    });

    if aggregate_battery.ReportUpdated(&handler).is_ok() {
        info!("[Battery Monitor] 成功订阅 WinRT Battery.ReportUpdated 事件。");
        std::future::pending::<()>().await;
    } else {
        error!("[Battery Monitor] 订阅 WinRT Battery.ReportUpdated 事件失败。");
    }
}

async fn setup_network_monitor(sender: mpsc::Sender<SystemEvent>) {
    info!("[Network Monitor] 任务启动。");
    let get_details = || -> windows::core::Result<Option<(String, ConnectionType)>> {
        let profile = NetworkInformation::GetInternetConnectionProfile()?;
        let name = profile.ProfileName()?.to_string();
        let iana_type = profile.NetworkAdapter()?.IanaInterfaceType()?;
        let conn_type = match iana_type { 6 => ConnectionType::Ethernet, 71 => ConnectionType::WiFi, 243 | 244 => ConnectionType::Cellular, _ => ConnectionType::Unknown };
        Ok(Some((name, conn_type)))
    };

    let last_state = Arc::new(Mutex::new(get_details().ok().flatten()));

    let handler = NetworkStatusChangedEventHandler::new({
        let sender_clone = sender.clone();
        let state_clone = last_state.clone();
        move |_| {
            if *IS_SYSTEM_ASLEEP.lock().unwrap() { return Ok(()); }
            info!("[Network Monitor] NetworkStatusChanged 事件触发。");
            
            let current_details = get_details()?;
            let mut last_details_guard = state_clone.lock().unwrap();

            if *last_details_guard != current_details {
                if last_details_guard.is_some() { 
                    if let Err(e) = sender_clone.send(SystemEvent::NetworkDisconnected) {
                        error!("发送 NetworkDisconnected 事件失败: {}", e);
                    }
                }
                if let Some((name, conn_type)) = &current_details {
                    let event = SystemEvent::NetworkConnected { name: name.clone(), conn_type: conn_type.clone() };
                    if let Err(e) = sender_clone.send(event) {
                        error!("发送 NetworkConnected 事件失败: {}", e);
                    }
                }
                *last_details_guard = current_details;
            }
            Ok(())
        }
    });

    if let Ok(_) = NetworkInformation::NetworkStatusChanged(&handler) {
        info!("[Network Monitor] 成功订阅 WinRT NetworkStatusChanged 事件。");
        std::future::pending::<()>().await;
    } else {
        error!("[Network Monitor] 订阅 WinRT NetworkStatusChanged 事件失败。");
    }
}