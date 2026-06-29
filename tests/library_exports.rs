use computer_use_linux::diagnostics::{Check, DoctorReport};

#[test]
fn exposes_diagnostics_types_to_library_consumers() {
    assert!(std::any::type_name::<Check>().contains("Check"));
    assert!(std::any::type_name::<DoctorReport>().contains("DoctorReport"));
}
