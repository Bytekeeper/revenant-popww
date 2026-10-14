//! EAX through DSOAL: `EAX.DLL` creates DirectSound via COM, which always loads the system
//! `dsound.dll` and so ignores a DSOAL `dsound.dll` in the game folder; we hand COM's request to
//! DSOAL under its own name instead, which needs no DLL override on Wine.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::core::{GUID, w};

use crate::config;
use crate::d3d::{Interface, RELEASE, Release, hook, method};
use crate::log::log;

const CLSID_DIRECT_SOUND: GUID = GUID::from_u128(0x47d4d946_62e8_11cf_93bc_444553540000);
const CLSID_DIRECT_SOUND8: GUID = GUID::from_u128(0x3901cc3f_84b5_4fa4_ba35_aa8172b8a09b);
const IID_ICLASS_FACTORY: GUID = GUID::from_u128(0x00000001_0000_0000_c000_000000000046);
const CLASS_FACTORY_CREATE_INSTANCE: usize = 3;
const E_FAIL: i32 = 0x80004005u32 as i32;

type CoCreateInstance = unsafe extern "system" fn(*const GUID, *mut c_void, u32, *const GUID, *mut *mut c_void) -> i32;
type DllGetClassObject = unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> i32;
type CreateInstance = unsafe extern "system" fn(Interface, *mut c_void, *const GUID, *mut *mut c_void) -> i32;

static ORIGINAL_CO_CREATE_INSTANCE: AtomicUsize = AtomicUsize::new(0);

/// After the game's imports are loaded; DirectSound starts long after.
pub fn install() {
    if !config::get().eax {
        return;
    }
    let ole32 = unsafe { LoadLibraryW(w!("ole32.dll")) };
    match (!ole32.is_null()).then(|| unsafe { GetProcAddress(ole32, c"CoCreateInstance".as_ptr().cast()) }).flatten() {
        Some(create) => unsafe {
            hook(create as *mut c_void, co_create_instance as *mut c_void, &ORIGINAL_CO_CREATE_INSTANCE, c"CoCreateInstance")
        },
        None => log!("CoCreateInstance not found, no EAX"),
    }
}

unsafe extern "system" fn co_create_instance(
    class: *const GUID,
    outer: *mut c_void,
    context: u32,
    interface: *const GUID,
    out: *mut *mut c_void,
) -> i32 {
    if !class.is_null() && is_direct_sound(unsafe { &*class }) {
        let result = unsafe { create_with_dsoal(class, outer, interface, out) };
        if result >= 0 {
            return result;
        }
    }
    let original: CoCreateInstance = unsafe { std::mem::transmute(ORIGINAL_CO_CREATE_INSTANCE.load(Ordering::Relaxed)) };
    unsafe { original(class, outer, context, interface, out) }
}

fn is_direct_sound(class: &GUID) -> bool {
    [CLSID_DIRECT_SOUND, CLSID_DIRECT_SOUND8]
        .iter()
        .any(|c| (c.data1, c.data2, c.data3, c.data4) == (class.data1, class.data2, class.data3, class.data4))
}

unsafe fn create_with_dsoal(class: *const GUID, outer: *mut c_void, interface: *const GUID, out: *mut *mut c_void) -> i32 {
    let Some(get_class_object) = dsoal() else {
        return E_FAIL;
    };
    let mut factory: Interface = std::ptr::null_mut();
    let mut result = unsafe { get_class_object(class, &IID_ICLASS_FACTORY, (&raw mut factory).cast()) };
    if result >= 0 {
        result = unsafe { method::<CreateInstance>(factory, CLASS_FACTORY_CREATE_INSTANCE)(factory, outer, interface, out) };
        unsafe { method::<Release>(factory, RELEASE)(factory) };
    }
    log!("DirectSound through DSOAL: {result:#x}");
    result
}

/// DSOAL's class factory, loaded on first use; None if it is missing or fails to load.
fn dsoal() -> Option<DllGetClassObject> {
    static GET_CLASS_OBJECT: AtomicUsize = AtomicUsize::new(usize::MAX);
    let mut address = GET_CLASS_OBJECT.load(Ordering::Relaxed);
    if address == usize::MAX {
        address = load_dsoal();
        GET_CLASS_OBJECT.store(address, Ordering::Relaxed);
    }
    (address != 0).then(|| unsafe { std::mem::transmute::<usize, DllGetClassObject>(address) })
}

fn load_dsoal() -> usize {
    use std::os::windows::ffi::OsStrExt;
    let path = crate::module_dir().join(DSOAL_FILE);
    if !path.exists() {
        log!("{DSOAL_FILE} not found, no EAX");
        return 0;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    let address = (!module.is_null())
        .then(|| unsafe { GetProcAddress(module, c"DllGetClassObject".as_ptr().cast()) })
        .flatten()
        .map_or(0, |f| f as usize);
    if address == 0 {
        log!("cannot load {}, no EAX", path.display());
    }
    address
}

/// DSOAL's `dsound.dll`, renamed so that nothing else loads it in place of the system's.
pub const DSOAL_FILE: &str = "popww-dsoal.dll";
