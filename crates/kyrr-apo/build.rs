fn main() {
    // Product name and version in kyrr_apo.dll, as in kyrr.exe: the code
    // signing service checks them. Only when building on Windows.
    if cfg!(windows) && std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set("ProductName", "kyrr");
        res.set("FileDescription", "kyrr audio filter");
        res.set("LegalCopyright", "Copyright (c) 2026 heast13");
        res.compile().expect("Windows resources compile");
    }
}
