extern crate configure_me_codegen;

fn main() {
    if let Some(mut man_dir) = std::env::var_os("MAN_DIR") {
        std::fs::create_dir_all(&man_dir).expect("Failed to create directory target/man");
        man_dir.push("btc_rpc_proxy.1");
        configure_me_codegen::build_script_with_man_written_to("config_spec.toml", man_dir)
    } else {
        configure_me_codegen::build_script("config_spec.toml")
    }.expect("Failed to generate code");
}
