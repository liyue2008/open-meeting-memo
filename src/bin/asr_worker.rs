fn main() {
    // The asr component supplies this synchronous process entrypoint.
    if let Err(error) = omm::adapters::asr::worker_main() {
        eprintln!("{}", error.message);
        std::process::exit(error.exit_code());
    }
}
