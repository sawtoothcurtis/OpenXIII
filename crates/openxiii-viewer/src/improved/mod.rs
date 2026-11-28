//! Compile the selected viewer extension at each subsystem's private boundary.

#[cfg(feature = "improved")]
macro_rules! register_viewer_layer {
    ($domain:literal) => {
        include!(concat!("improved/enabled/", $domain, ".rs"));
    };
}

#[cfg(not(feature = "improved"))]
macro_rules! register_viewer_layer {
    ($domain:literal) => {
        include!(concat!("improved/faithful/", $domain, ".rs"));
    };
}
