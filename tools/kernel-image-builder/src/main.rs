use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bootloader::{BiosBoot, UefiBoot};

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let kernel = args
        .next()
        .context("missing kernel binary path argument")?;
    let output_dir = args
        .next()
        .context("missing output directory argument")?;

    let kernel = PathBuf::from(kernel);
    let output_dir = PathBuf::from(output_dir);
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let uefi_image = output_dir.join("waros.img");
    let bios_image = output_dir.join("waros-bios.img");

    create_uefi_image(&kernel, &uefi_image)?;
    create_bios_image(&kernel, &bios_image)?;

    println!("Created UEFI image: {}", uefi_image.display());
    println!("Created BIOS image: {}", bios_image.display());
    Ok(())
}

fn create_uefi_image(kernel: &Path, output: &Path) -> Result<()> {
    UefiBoot::new(kernel)
        .create_disk_image(output)
        .with_context(|| format!("failed to create UEFI image {}", output.display()))
}

fn create_bios_image(kernel: &Path, output: &Path) -> Result<()> {
    BiosBoot::new(kernel)
        .create_disk_image(output)
        .with_context(|| format!("failed to create BIOS image {}", output.display()))
}
