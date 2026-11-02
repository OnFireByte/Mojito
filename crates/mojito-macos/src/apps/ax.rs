use std::ptr::{self, NonNull};

use objc2_application_services::{AXError, AXUIElement};
use objc2_core_foundation::{CFArray, CFBoolean, CFRetained, CFString, CFType, ConcreteType};

use crate::cf::cf_items;

const MESSAGING_TIMEOUT_SECS: f32 = 0.25;

pub(crate) struct AxElement(CFRetained<AXUIElement>);

impl AxElement {
    pub(crate) fn application(pid: i32) -> Option<Self> {
        // SAFETY: AXUIElementCreateApplication accepts any pid; an unknown
        // pid yields an element whose queries fail, never invalid memory.
        Self::bounded(unsafe { AXUIElement::new_application(pid) })
    }

    pub(crate) fn element(&self, attribute: &'static str) -> Option<Self> {
        Self::bounded(self.copy::<AXUIElement>(attribute)?)
    }

    pub(crate) fn elements(&self, attribute: &'static str) -> Vec<Self> {
        let Some(array) = self.copy::<CFArray>(attribute) else {
            return Vec::new();
        };
        cf_items::<AXUIElement>(array)
            .into_iter()
            .filter_map(Self::bounded)
            .collect()
    }

    pub(crate) fn bool_attribute(&self, attribute: &'static str) -> Option<bool> {
        self.copy::<CFBoolean>(attribute)
            .map(|value| value.as_bool())
    }

    pub(crate) fn set_bool_if_settable(&self, attribute: &'static str, value: bool) {
        let attribute = CFString::from_static_str(attribute);
        let mut settable = 0_u8;
        // SAFETY: `settable` is a live local the call writes one byte into.
        let status = unsafe {
            self.0
                .is_attribute_settable(&attribute, NonNull::from(&mut settable))
        };
        if status != AXError::Success || settable == 0 {
            return;
        }
        // SAFETY: a CFBoolean is the value type of the boolean attributes
        // this is called with.
        unsafe {
            self.0
                .set_attribute_value(&attribute, CFBoolean::new(value))
        };
    }

    pub(crate) fn perform(&self, action: &'static str) -> Result<(), String> {
        // SAFETY: the element and the action name are live CF objects.
        let status = unsafe { self.0.perform_action(&CFString::from_static_str(action)) };
        if status == AXError::Success {
            Ok(())
        } else {
            Err(format!("{action} failed (status {})", status.0))
        }
    }

    fn bounded(element: CFRetained<AXUIElement>) -> Option<Self> {
        // SAFETY: setting a timeout on a live element has no other preconditions.
        let status = unsafe { element.set_messaging_timeout(MESSAGING_TIMEOUT_SECS) };
        (status == AXError::Success).then_some(Self(element))
    }

    fn copy<T: ConcreteType>(&self, attribute: &'static str) -> Option<CFRetained<T>> {
        let attribute = CFString::from_static_str(attribute);
        let mut value: *const CFType = ptr::null();
        // SAFETY: `value` is a live local that receives a +1 reference on
        // success, which `from_raw` adopts exactly once.
        let value = unsafe {
            let status = self
                .0
                .copy_attribute_value(&attribute, NonNull::from(&mut value));
            if status != AXError::Success {
                return None;
            }
            CFRetained::from_raw(NonNull::new(value.cast_mut())?)
        };
        value.downcast::<T>().ok()
    }
}
