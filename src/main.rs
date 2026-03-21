fn main() {
    #[cfg(feature = "ssr")]
    {
        mcbean::server::run()
    }
    #[cfg(not(feature = "ssr"))]
    {
        panic!("ssr feature is required to run the server binary");
    }
}
