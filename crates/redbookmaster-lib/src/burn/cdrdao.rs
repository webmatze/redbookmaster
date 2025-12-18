// CD burning via cdrdao
// Provides integration with the cdrdao command-line tool

use std::path::{Path, PathBuf};
use std::process::Command;

/// Common paths where cdrdao might be installed
const CDRDAO_PATHS: &[&str] = &[
    "cdrdao",                           // In PATH
    "/opt/homebrew/bin/cdrdao",         // Homebrew on Apple Silicon
    "/usr/local/bin/cdrdao",            // Homebrew on Intel Mac / manual install
    "/usr/bin/cdrdao",                  // Linux system install
    "/bin/cdrdao",                      // Alternative Linux location
];

/// Find the cdrdao binary path
pub fn find_cdrdao() -> Option<PathBuf> {
    for path in CDRDAO_PATHS {
        let path = PathBuf::from(path);

        // For "cdrdao" (no path), try executing it directly
        if path.as_os_str() == "cdrdao" {
            if Command::new("cdrdao")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return Some(path);
            }
        } else if path.exists() {
            // For absolute paths, check if file exists
            return Some(path);
        }
    }
    None
}

/// Get the cdrdao command (finds the binary automatically)
fn cdrdao_command() -> Option<Command> {
    find_cdrdao().map(Command::new)
}

/// Check if cdrdao is available on the system
pub fn is_available() -> bool {
    find_cdrdao().is_some()
}

/// Get cdrdao version string
pub fn version() -> Option<String> {
    let mut cmd = cdrdao_command()?;
    cmd.arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                String::from_utf8(o.stderr).ok().or_else(|| String::from_utf8(o.stdout).ok())
            } else {
                None
            }
        })
        .map(|s| s.lines().next().unwrap_or("unknown").to_string())
}

/// CD drive information
#[derive(Debug, Clone)]
pub struct CdDrive {
    pub device: String,
    pub vendor: String,
    pub model: String,
}

/// List available CD drives
pub fn list_drives() -> Result<Vec<CdDrive>, CdrdaoError> {
    let mut cmd = cdrdao_command().ok_or(CdrdaoError::NotInstalled)?;
    let output = cmd
        .args(["scanbus"])
        .output()
        .map_err(|e| CdrdaoError::ExecutionError(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CdrdaoError::CommandFailed(stderr.to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut drives = Vec::new();

    // Parse scanbus output
    // Format: X,Y,Z : Vendor Model
    for line in stdout.lines() {
        if let Some((device, rest)) = line.split_once(':') {
            let device = device.trim().to_string();
            let parts: Vec<&str> = rest.trim().splitn(2, ' ').collect();
            if parts.len() >= 2 {
                drives.push(CdDrive {
                    device,
                    vendor: parts[0].to_string(),
                    model: parts[1].to_string(),
                });
            }
        }
    }

    Ok(drives)
}

/// Burn options
#[derive(Debug, Clone)]
pub struct BurnOptions {
    /// CD drive device path
    pub device: String,
    /// Burn speed (0 = auto)
    pub speed: u32,
    /// Simulate burn (don't actually write)
    pub simulate: bool,
    /// Eject disc after burning
    pub eject: bool,
    /// Use generic-mmc-raw driver for CD-TEXT support
    pub force_raw_driver: bool,
}

impl Default for BurnOptions {
    fn default() -> Self {
        Self {
            device: "/dev/sr0".to_string(),
            speed: 0,
            simulate: false,
            eject: true,
            force_raw_driver: true,
        }
    }
}

/// Cdrdao wrapper
pub struct Cdrdao {
    options: BurnOptions,
}

impl Cdrdao {
    pub fn new(options: BurnOptions) -> Self {
        Self { options }
    }

    /// Burn a TOC file to CD
    pub fn burn(&self, toc_file: &Path) -> Result<(), CdrdaoError> {
        let mut args = vec!["write"];

        // Device
        args.push("--device");
        args.push(&self.options.device);

        // Driver for CD-TEXT support
        if self.options.force_raw_driver {
            args.push("--driver");
            args.push("generic-mmc-raw");
        }

        // Speed
        if self.options.speed > 0 {
            args.push("--speed");
            let speed_str = self.options.speed.to_string();
            args.push(Box::leak(speed_str.into_boxed_str()));
        }

        // Simulate
        if self.options.simulate {
            args.push("--simulate");
        }

        // Eject
        if self.options.eject {
            args.push("--eject");
        }

        // Verbose output
        args.push("-v");
        args.push("2");

        // TOC file
        let toc_str = toc_file.to_string_lossy();
        args.push(&toc_str);

        let mut cmd = cdrdao_command().ok_or(CdrdaoError::NotInstalled)?;
        let output = cmd
            .args(&args)
            .output()
            .map_err(|e| CdrdaoError::ExecutionError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CdrdaoError::BurnFailed(stderr.to_string()));
        }

        Ok(())
    }

    /// Simulate burning (dry run)
    pub fn simulate(&self, toc_file: &Path) -> Result<(), CdrdaoError> {
        let mut sim_options = self.options.clone();
        sim_options.simulate = true;

        let sim_cdrdao = Cdrdao::new(sim_options);
        sim_cdrdao.burn(toc_file)
    }

    /// Read disc info
    pub fn disc_info(&self) -> Result<String, CdrdaoError> {
        let mut cmd = cdrdao_command().ok_or(CdrdaoError::NotInstalled)?;
        let output = cmd
            .args(["disk-info", "--device", &self.options.device])
            .output()
            .map_err(|e| CdrdaoError::ExecutionError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CdrdaoError::CommandFailed(stderr.to_string()));
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Blank a CD-RW disc
    pub fn blank(&self) -> Result<(), CdrdaoError> {
        let mut cmd = cdrdao_command().ok_or(CdrdaoError::NotInstalled)?;
        let output = cmd
            .args(["blank", "--device", &self.options.device, "--blank-mode", "minimal"])
            .output()
            .map_err(|e| CdrdaoError::ExecutionError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(CdrdaoError::CommandFailed(stderr.to_string()));
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CdrdaoError {
    #[error("cdrdao is not installed. Please install it to burn CDs.")]
    NotInstalled,

    #[error("Failed to execute cdrdao: {0}")]
    ExecutionError(String),

    #[error("cdrdao command failed: {0}")]
    CommandFailed(String),

    #[error("Burn failed: {0}")]
    BurnFailed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let options = BurnOptions::default();
        assert!(options.eject);
        assert!(options.force_raw_driver);
        assert!(!options.simulate);
    }

    #[test]
    fn test_find_cdrdao() {
        // This test verifies cdrdao can be found if installed
        let found = find_cdrdao();
        if found.is_some() {
            println!("Found cdrdao at: {:?}", found.unwrap());
            assert!(is_available());
        }
    }
}
