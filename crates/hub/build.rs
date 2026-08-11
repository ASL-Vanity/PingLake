use std::{env, path::Path};

fn main() {
    println!("cargo:rustc-check-cfg=cfg(web_dist_present)");
    println!("cargo:rerun-if-changed=../../web/dist");

    let web_dist_present = Path::new("../../web/dist/index.html").is_file();
    if web_dist_present {
        println!("cargo:rustc-cfg=web_dist_present");
    } else if env::var("PROFILE").as_deref() == Ok("release") {
        panic!(
            "web/dist/index.html is missing; run `npm ci && npm run build` in web before building a release Hub"
        );
    }
}
