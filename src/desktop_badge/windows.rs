//! Small native resource owner attached to the actual Win32 window. Reapply the
//! latest prepared image when Explorer recreates its taskbar button. No mail state
//! or cross-thread locks live in the window procedure.
use super::overlay::{Frame, SIZE};
use std::sync::Arc;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RPC_E_CHANGED_MODE, WPARAM},
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::{
            Shell::{
                DefSubclassProc, GetWindowSubclass, ITaskbarList3, RemoveWindowSubclass,
                SetWindowSubclass, TaskbarList,
            },
            WindowsAndMessaging::{
                CreateIcon, DestroyIcon, HICON, RegisterWindowMessageW, WM_NCDESTROY,
            },
        },
    },
    core::{PCWSTR, w},
};
const SUBCLASS: usize = 0x53484550;
struct Owner {
    frame: Arc<Frame>,
    created_message: u32,
}
struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        let _ = unsafe { DestroyIcon(self.0) };
    }
}
struct Apartment(bool);
impl Drop for Apartment {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() }
        }
    }
}

fn display(window: HWND, frame: &Frame) -> anyhow::Result<()> {
    // Balance successful initialization, including S_FALSE; an existing different
    // COM apartment is owned by iced and must not be uninitialized here.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized != RPC_E_CHANGED_MODE {
        initialized.ok()?;
    }
    let _apartment = Apartment(initialized.is_ok());
    let taskbar: ITaskbarList3 =
        unsafe { CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)? };
    unsafe {
        taskbar.HrInit()?;
    }
    let icon = if frame.count > 0 {
        let mut bgra = frame.rgba.clone();
        for pixel in bgra.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        // 1-bit AND mask rows are word-aligned. Alpha is in the 32-bit color data.
        let mask = [0u8; SIZE * (SIZE / 8)];
        Some(Icon(unsafe {
            CreateIcon(
                None,
                SIZE as i32,
                SIZE as i32,
                1,
                32,
                mask.as_ptr(),
                bgra.as_ptr(),
            )?
        }))
    } else {
        None
    };
    let description: Vec<u16> = frame.description.encode_utf16().chain(Some(0)).collect();
    unsafe {
        taskbar.SetOverlayIcon(
            window,
            icon.as_ref().map(|icon| icon.0).unwrap_or_default(),
            PCWSTR(description.as_ptr()),
        )?;
    }
    Ok(())
}

pub(super) fn apply(raw_window: isize, frame: Arc<Frame>) -> anyhow::Result<()> {
    let window = HWND(raw_window as *mut _);
    let mut data = 0usize;
    // iced calls this on the owning event thread; the subclass data is never
    // accessed from a worker. WM_NCDESTROY releases it when tray closes a window.
    unsafe {
        if GetWindowSubclass(window, Some(callback), SUBCLASS, Some(&mut data)).as_bool() {
            (*(data as *mut Owner)).frame = frame.clone();
        } else {
            let created_message = RegisterWindowMessageW(w!("TaskbarButtonCreated"));
            anyhow::ensure!(
                created_message != 0,
                "Cannot register taskbar recovery message"
            );
            let owner = Box::new(Owner {
                frame: frame.clone(),
                created_message,
            });
            let raw = Box::into_raw(owner);
            if !SetWindowSubclass(window, Some(callback), SUBCLASS, raw as usize).as_bool() {
                drop(Box::from_raw(raw));
                anyhow::bail!("Cannot attach taskbar recovery handler");
            }
        }
    }
    // The native message handles a button created after this initial attempt.
    display(window, &frame)
}

unsafe extern "system" fn callback(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let owner = data as *mut Owner;
    if message == WM_NCDESTROY {
        unsafe {
            let _ = RemoveWindowSubclass(window, Some(callback), SUBCLASS);
            drop(Box::from_raw(owner));
        }
    } else if message == unsafe { (*owner).created_message } {
        let frame = unsafe { (*owner).frame.clone() };
        if let Err(error) = display(window, &frame) {
            tracing::debug!(%error, "Taskbar badge could not be restored");
        }
    }
    unsafe { DefSubclassProc(window, message, wparam, lparam) }
}
