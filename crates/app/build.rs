fn main() {
    println!("cargo:rerun-if-changed=assets/icon/rword.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon/rword.ico");
        res.set("ProductName", "RWord");
        res.set("FileDescription", "RWord word processor");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows icon: {}", e);
        }
    }
}
