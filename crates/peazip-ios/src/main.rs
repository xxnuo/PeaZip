#[cfg(target_os = "ios")]
fn main() {
    nsh_platform_ios::set_ui_sets(vec![nsh_base::registry(), peazip_ui::__nsh_native_module()]);
    nsh_ios_app::run_rust(nsh_core::element::Root::of::<peazip_ui::App>().app_id("io.peazip.rust"));
}

#[cfg(not(target_os = "ios"))]
fn main() {}
