use std::iter;

use windows_sys::Win32::Graphics::Gdi::{CDS_FULLSCREEN, CDS_TEST, ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_BITSPERPEL, DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH};
use winit::monitor::VideoModeHandle;

pub(super) fn mode_supported(video_mode: &VideoModeHandle) -> bool {
    let monitor = video_mode.monitor();
    let Some(name) = monitor.name() else {
        return false;
    };
    let device_name: Vec<u16> = name.encode_utf16().chain(iter::once(0)).collect();
    let size = video_mode.size();
    let settings = DEVMODEW {
        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
        dmPelsWidth: size.width,
        dmPelsHeight: size.height,
        dmBitsPerPel: u32::from(video_mode.bit_depth()),
        dmDisplayFrequency: video_mode.refresh_rate_millihertz() / 1000,
        dmFields: DM_PELSWIDTH | DM_PELSHEIGHT | DM_BITSPERPEL | DM_DISPLAYFREQUENCY,
        ..Default::default()
    };

    let result = unsafe { ChangeDisplaySettingsExW(device_name.as_ptr(), &settings, std::ptr::null_mut(), CDS_FULLSCREEN | CDS_TEST, std::ptr::null()) };
    result == DISP_CHANGE_SUCCESSFUL
}
