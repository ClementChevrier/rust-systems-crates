#![allow(clippy::unwrap_used, clippy::panic)]

use super::*;

/// Puts the original class back even if an assertion fails midway.
struct RestoreClass(ProcessClass);
impl Drop for RestoreClass {
    fn drop(&mut self) {
        let _ = set_class(self.0);
    }
}

/// Must stay the only test that touches the process class: it is global, and
/// two concurrent tests changing it would step on each other.
#[test]
fn set_class_is_observable_and_restorable() {
    let original = class().expect("class must be readable");
    let _restore = RestoreClass(original);

    set_class(ProcessClass::BelowNormal).expect("change accepted");
    assert_eq!(
        class().expect("class must be readable"),
        ProcessClass::BelowNormal
    );

    // Without the privilege Windows grants High instead: that must come back
    // as an error, never as a silent success.
    match set_class(ProcessClass::RealTime) {
        Ok(()) => assert_eq!(
            class().expect("class must be readable"),
            ProcessClass::RealTime
        ),
        Err(err) => assert_eq!(err.kind(), io::ErrorKind::PermissionDenied, "{err}"),
    }

    set_class(original).expect("restore");
    assert_eq!(class().expect("class must be readable"), original);
}
