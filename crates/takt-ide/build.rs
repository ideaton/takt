//! Icône de l'exécutable Windows.

fn main() {
    println!("cargo:rerun-if-changed=assets/takt.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/takt.ico");
        res.set("ProductName", "Takt Studio");
        res.set("FileDescription", "Takt Studio");
        if let Err(e) = res.compile() {
            println!("cargo:warning=icône non intégrée : {e}");
        }
    }
}
