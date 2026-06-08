// naga (Bevy's shader compiler) calls these C math symbols which don't exist in
// wasm32-unknown-unknown's standard library. Provide them via Rust's built-in float methods.
#[cfg(target_arch = "wasm32")]
mod math_shims {
    #[unsafe(no_mangle)]
    pub extern "C" fn asinh(x: f64) -> f64 {
        x.asinh()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn asinhf(x: f32) -> f32 {
        x.asinh()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn acosh(x: f64) -> f64 {
        x.acosh()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn acoshf(x: f32) -> f32 {
        x.acosh()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn atanh(x: f64) -> f64 {
        x.atanh()
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn atanhf(x: f32) -> f32 {
        x.atanh()
    }
}
