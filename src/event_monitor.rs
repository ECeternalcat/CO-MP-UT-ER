// src/event_monitor.rs

use std::sync::{mpsc, Arc, Mutex};
use log::{info, error};
use windows::core::{IInspectable};
use windows::Foundation::{TypedEventHandler, IReference};
use windows::Devices::Power::Battery;
use windows::Networking::Connectivity::{NetworkInformation, NetworkStatusChangedEventHandler};
use windows::Win32::Foundation::{HWND, WPARAM, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
// --- Add c_void for the explicit cast ---
use std::ffi::c_void;

lazy_static::lazy_static! {
    pub static ref IS_SYSTEM_ASLEEP: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
}
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use futures::executor::block_on;

const WM_APP_WAKEUP: u32 = 0x8000 + 2;

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionType { Ethernet, WiFi, Cellular, Unknown }

#[derive(Debug)]
pub enum SystemEvent {
    PowerSwitchedToAC, PowerSwitchedToBattery,
    BatteryLevelReport(u8),
    UsbDeviceConnected, UsbDeviceDisconnected, SystemStartup,
    BatteryInserted, BatteryRemoved,
    NetworkConnected { name: String, conn_type: ConnectionType },
    NetworkDisconnected,
    SystemGoingToSleep,
    SystemResumedFromSleep,
}

// The public API still takes an HWND for clarity.
pub fn start_monitoring(sender: mpsc::Sender<SystemEvent>, hwnd: HWND) {
    // --- CORE FIX: Cast the raw pointer (*mut c_void) to a pointer-sized integer (isize). ---
    // This is safe because isize is guaranteed to be large enough to hold a pointer.
    // The isize value is `Send` and can be moved to other threads.
    let hwnd_value = hwnd.0 as isize;

    let battery_sender = sender.clone();
    std::thread::spawn(move || {
        if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok() {
            // Pass the isize value, not the HWND.
            block_on(setup_battery_monitor(battery_sender, hwnd_value));
        }
    });

    let network_sender = sender;
    std::thread::spawn(move || {
        if unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok() {
            // Pass the isize value, not the HWND.
            block_on(setup_network_monitor(network_sender, hwnd_value));
        }
    });
}

// This function correctly accepts the raw isize value.
async fn setup_battery_monitor(sender: mpsc::Sender<SystemEvent>, hwnd_value: isize) {
    let aggregate_battery = match Battery::AggregateBattery() {
        Ok(b) => b,
        Err(_) => return
    };

    let last_present_state = Arc::new(Mutex::new(None::<bool>));
    let last_percentage = Arc::new(Mutex::new(None::<u8>));

    if let Ok(report) = aggregate_battery.GetReport() {
        let is_present = report.FullChargeCapacityInMilliwattHours()
            .and_then(|cap| cap.GetInt32())
            .map_or(false, |c| c > 0);
        *last_present_state.lock().unwrap() = Some(is_present);

        if let (Ok(rem_cap), Ok(full_cap)) = 
            (report.RemainingCapacityInMilliwattHours(), report.FullChargeCapacityInMilliwattHours()) {
            if let (Ok(rem), Ok(full)) = (rem_cap.GetInt32(), full_cap.GetInt32()) {
                if full > 0 {
                    let percentage = (rem as f64 / full as f64 * 100.0).round() as u8;
                    *last_percentage.lock().unwrap() = Some(percentage);
                }
            }
        }
    }

    let handler = TypedEventHandler::<Battery, IInspectable>::new({
        let sender_clone = sender.clone();
        let state_clone = last_present_state.clone();
        let percentage_clone = last_percentage.clone();
        let battery_clone = aggregate_battery.clone(); 
        
        move |_, _| {
            if *IS_SYSTEM_ASLEEP.lock().unwrap() { return Ok(()); }
            
            let report = match battery_clone.GetReport() { Ok(r) => r, Err(_) => return Ok(()) };

            let is_present_now = report.FullChargeCapacityInMilliwattHours().and_then(|c| c.GetInt32()).map_or(false, |c| c > 0);

            let percentage_now = if let (Ok(rem_cap), Ok(full_cap)) = (report.RemainingCapacityInMilliwattHours(), report.FullChargeCapacityInMilliwattHours()) {
                if let (Ok(rem), Ok(full)) = (rem_cap.GetInt32(), full_cap.GetInt32()) {
                    if full > 0 { Some((rem as f64 / full as f64 * 100.0).round() as u8) } else { None }
                } else { None }
            } else { None };
            
            let mut last_present_guard = state_clone.lock().unwrap();
            let mut last_percentage_guard = percentage_clone.lock().unwrap();
            
            let mut event_to_send: Option<SystemEvent> = None;

            if *last_present_guard != Some(is_present_now) {
                event_to_send = Some(if is_present_now { SystemEvent::BatteryInserted } else { SystemEvent::BatteryRemoved });
                *last_present_guard = Some(is_present_now);
                *last_percentage_guard = percentage_now;
            } else if is_present_now && *last_percentage_guard != percentage_now && percentage_now.is_some() {
                event_to_send = Some(SystemEvent::BatteryLevelReport(percentage_now.unwrap()));
                *last_percentage_guard = percentage_now;
            }

            if let Some(event) = event_to_send {
                if sender_clone.send(event).is_ok() {
                    // --- CORE FIX: Cast the isize back to a raw pointer and then create the HWND. ---
                    let hwnd = HWND(hwnd_value as *mut c_void);
                    unsafe { PostMessageW(Some(hwnd), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
                }
            }

            Ok(())
        }
    });

    if aggregate_battery.ReportUpdated(&handler).is_ok() {
        std::future::pending::<()>().await;
    }
}

// This function correctly accepts the raw isize value.
async fn setup_network_monitor(sender: mpsc::Sender<SystemEvent>, hwnd_value: isize) {
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
            
            let current_details = get_details()?;
            let mut last_details_guard = state_clone.lock().unwrap();

            if *last_details_guard != current_details {
                // --- CORE FIX: Cast the isize back to a raw pointer and then create the HWND. ---
                let hwnd = HWND(hwnd_value as *mut c_void);

                if last_details_guard.is_some() { 
                    if sender_clone.send(SystemEvent::NetworkDisconnected).is_ok() {
                        unsafe { PostMessageW(Some(hwnd), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
                    }
                }
                if let Some((name, conn_type)) = &current_details {
                    let event = SystemEvent::NetworkConnected { name: name.clone(), conn_type: conn_type.clone() };
                    if sender_clone.send(event).is_ok() {
                        unsafe { PostMessageW(Some(hwnd), WM_APP_WAKEUP, WPARAM(0), LPARAM(0)).ok(); }
                    }
                }
                *last_details_guard = current_details;
            }
            Ok(())
        }
    });

    if NetworkInformation::NetworkStatusChanged(&handler).is_ok() {
        std::future::pending::<()>().await;
    }
}