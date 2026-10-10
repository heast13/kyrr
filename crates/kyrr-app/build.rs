fn main() {
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("ui/lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("ui compiles");

    // Only when building on Windows: the resource compiler is part of the
    // Windows SDK, which cross builds from Linux do not have.
    if cfg!(windows) && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("ui/icons/kyrr.ico");
        res.set("ProductName", "kyrr");
        res.set("FileDescription", "kyrr");
        res.set("LegalCopyright", "Copyright (c) 2026 heast13");
        res.compile().expect("Windows resources compile");
    }
}
