#[cfg(target_os = "android")]
nsh::ui_sets!(vec![nsh_base::registry(), peazip_ui::__nsh_native_module()]);

#[cfg(target_os = "android")]
nsh::main!(nsh_core::element::Root::of::<peazip_ui::App>().app_id("io.peazip.rust"));
