use objc2_core_foundation::{CFArray, CFDictionary, CFRetained, CFString, CFType, ConcreteType};

pub(crate) type CFStringDict = CFDictionary<CFString, CFType>;

pub(crate) fn cf_items<T: ConcreteType>(array: CFRetained<CFArray>) -> Vec<CFRetained<T>> {
    // SAFETY: every element of a CFArray is a CFType; each one is
    // type-checked with `downcast` before use.
    let array = unsafe { CFRetained::cast_unchecked::<CFArray<CFType>>(array) };
    array
        .iter()
        .filter_map(|item| item.downcast::<T>().ok())
        .collect()
}

pub(crate) fn cf_dicts(array: CFRetained<CFArray>) -> Vec<CFRetained<CFStringDict>> {
    cf_items::<CFDictionary>(array)
        .into_iter()
        .map(|dict| {
            // SAFETY: lookups only hash and compare keys (CFHash/CFEqual,
            // defined for every CFType), and `cf_get` type-checks every value.
            unsafe { CFRetained::cast_unchecked::<CFStringDict>(dict) }
        })
        .collect()
}

pub(crate) fn cf_get<T: ConcreteType>(
    dict: &CFStringDict,
    key: &'static str,
) -> Option<CFRetained<T>> {
    dict.get(&CFString::from_static_str(key))?
        .downcast::<T>()
        .ok()
}
