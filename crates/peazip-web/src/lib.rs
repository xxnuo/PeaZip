#[cfg(target_arch = "wasm32")]
nsh_platform_web::rust_app!(
    vec![nsh_base::registry(), peazip_ui::__nsh_native_module()],
    nsh_core::element::Root::of::<peazip_ui::App>()
);
