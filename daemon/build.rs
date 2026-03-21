use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    rsbinder_aidl::Builder::new()
        .source("interface/mist/IMistService.aidl")
        .output("mist.rs")
        .generate()?;

    Ok(())
}
