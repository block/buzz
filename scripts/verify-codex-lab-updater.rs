use minisign_verify::{PublicKey, Signature};
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let public_key_path = args.next().ok_or("missing public key path")?;
    let signature_path = args.next().ok_or("missing signature path")?;
    let artifact_path = args.next().ok_or("missing artifact path")?;
    if args.next().is_some() {
        return Err("unexpected extra argument".into());
    }

    let public_key = PublicKey::from_file(public_key_path)?;
    let signature = Signature::from_file(signature_path)?;
    let mut verifier = public_key.verify_stream(&signature)?;
    let mut artifact = File::open(artifact_path)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = artifact.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    verifier.finalize()?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("updater signature verification failed: {error}");
        std::process::exit(1);
    }
}
