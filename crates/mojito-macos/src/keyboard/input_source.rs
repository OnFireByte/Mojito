use std::cell::UnsafeCell;
use std::marker::{PhantomData, PhantomPinned};
use std::ptr::NonNull;

use objc2::MainThreadMarker;
use objc2_core_foundation::{
    CFArray, CFBoolean, CFData, CFDictionary, CFRetained, CFString, CFType, cf_type,
};

#[repr(C)]
pub(crate) struct TISInputSource {
    inner: [u8; 0],
    _p: UnsafeCell<PhantomData<(*const UnsafeCell<()>, PhantomPinned)>>,
}

cf_type!(
    unsafe impl TISInputSource {}
);

// SAFETY: these signatures match HIToolbox's TextInputSources.h. The
// create/copy functions return +1 references (or null), which the callers
// below adopt with `CFRetained::from_raw`. `TISGetInputSourceProperty`
// returns a borrowed reference owned by the source (or null).
// `TISSelectInputSource` only reads the source. The statics are immutable
// CFString constants.
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCreateInputSourceList(
        properties: Option<&CFDictionary>,
        include_all_installed: bool,
    ) -> Option<NonNull<CFArray<TISInputSource>>>;
    fn TISCopyCurrentKeyboardInputSource() -> Option<NonNull<TISInputSource>>;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> Option<NonNull<TISInputSource>>;
    fn TISSelectInputSource(source: &TISInputSource) -> i32;
    fn TISGetInputSourceProperty(source: &TISInputSource, key: &CFString) -> *const CFType;
    static kTISPropertyInputSourceID: &'static CFString;
    static kTISPropertyLocalizedName: &'static CFString;
    static kTISPropertyInputSourceCategory: &'static CFString;
    static kTISPropertyInputSourceIsSelectCapable: &'static CFString;
    static kTISPropertyUnicodeKeyLayoutData: &'static CFString;
    static kTISCategoryKeyboardInputSource: &'static CFString;
    pub(crate) static kTISNotifySelectedKeyboardInputSourceChanged: &'static CFString;
}

#[derive(Clone, Copy)]
enum Property {
    Id,
    LocalizedName,
    Category,
    IsSelectCapable,
    UnicodeKeyLayoutData,
}

impl Property {
    fn key(self) -> &'static CFString {
        // SAFETY: reading immutable CFString constants exported by HIToolbox.
        unsafe {
            match self {
                Self::Id => kTISPropertyInputSourceID,
                Self::LocalizedName => kTISPropertyLocalizedName,
                Self::Category => kTISPropertyInputSourceCategory,
                Self::IsSelectCapable => kTISPropertyInputSourceIsSelectCapable,
                Self::UnicodeKeyLayoutData => kTISPropertyUnicodeKeyLayoutData,
            }
        }
    }
}

pub(crate) enum Scope {
    Enabled,
    Installed,
}

pub(crate) struct InputSource(CFRetained<TISInputSource>);

pub(crate) fn list(_mtm: MainThreadMarker, scope: Scope) -> Vec<InputSource> {
    let include_all = matches!(scope, Scope::Installed);
    // SAFETY: main thread (marker); a null filter lists every source, and
    // a TIS create function hands us a +1 array.
    let array =
        unsafe { TISCreateInputSourceList(None, include_all).map(|raw| CFRetained::from_raw(raw)) };
    array.map_or_else(Vec::new, |array| array.iter().map(InputSource).collect())
}

pub(crate) fn current_keyboard(_mtm: MainThreadMarker) -> Option<InputSource> {
    // SAFETY: main thread (marker); a TIS copy function hands us a +1 source.
    unsafe { TISCopyCurrentKeyboardInputSource().map(|raw| InputSource(CFRetained::from_raw(raw))) }
}

pub(crate) fn current_layout(_mtm: MainThreadMarker) -> Option<InputSource> {
    // SAFETY: as `current_keyboard`.
    unsafe {
        TISCopyCurrentKeyboardLayoutInputSource().map(|raw| InputSource(CFRetained::from_raw(raw)))
    }
}

impl InputSource {
    pub(crate) fn id(&self) -> Option<String> {
        self.string(Property::Id)
    }

    pub(crate) fn localized_name(&self) -> Option<String> {
        self.string(Property::LocalizedName)
    }

    pub(crate) fn is_keyboard(&self) -> bool {
        // SAFETY: reading an immutable CFString constant exported by HIToolbox.
        let keyboard = unsafe { kTISCategoryKeyboardInputSource };
        self.property::<CFString>(Property::Category)
            .is_some_and(|category| category == keyboard)
    }

    pub(crate) fn is_select_capable(&self) -> bool {
        self.property::<CFBoolean>(Property::IsSelectCapable)
            .is_some_and(CFBoolean::as_bool)
    }

    pub(crate) fn key_layout_data(&self) -> Option<&CFData> {
        self.property(Property::UnicodeKeyLayoutData)
    }

    pub(crate) fn select(&self) -> Result<(), String> {
        // SAFETY: `InputSource` exists only on the main thread, and the
        // source stays retained for the call.
        match unsafe { TISSelectInputSource(&self.0) } {
            0 => Ok(()),
            status => Err(format!("TISSelectInputSource failed with status {status}")),
        }
    }

    fn string(&self, property: Property) -> Option<String> {
        self.property::<CFString>(property).map(ToString::to_string)
    }

    fn property<T: objc2_core_foundation::ConcreteType>(&self, property: Property) -> Option<&T> {
        // SAFETY: main thread (`InputSource` is !Send); the returned value
        // is owned by the source, so borrowing it for `&self` keeps it alive.
        let value = unsafe { TISGetInputSourceProperty(&self.0, property.key()).as_ref() }?;
        value.downcast_ref::<T>()
    }
}
