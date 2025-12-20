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

/// CD-TEXT driver mode for burning
/// Different drives require different driver configurations for CD-TEXT support
#[derive(Debug, Clone, Default, PartialEq)]
pub enum CdTextDriver {
    /// No CD-TEXT - burn audio only without metadata
    Disabled,
    /// Let cdrdao auto-detect the driver (no --driver flag)
    #[default]
    Auto,
    /// Use generic-mmc-raw driver (common for CD-TEXT)
    GenericMmcRaw,
    /// Use generic-mmc with 0x10 flag (force R-W sub-channel writing)
    GenericMmcSubchannel,
    /// Use generic-mmc with 0x20000 flag (force CUE sheet, for some Pioneer drives)
    GenericMmcCueSheet,
}

impl CdTextDriver {
    /// Get the driver arguments for cdrdao
    pub fn driver_args(&self) -> Option<(&'static str, &'static str)> {
        match self {
            CdTextDriver::Disabled => None,
            CdTextDriver::Auto => None,
            CdTextDriver::GenericMmcRaw => Some(("--driver", "generic-mmc-raw")),
            CdTextDriver::GenericMmcSubchannel => Some(("--driver", "generic-mmc:0x10")),
            CdTextDriver::GenericMmcCueSheet => Some(("--driver", "generic-mmc:0x20000")),
        }
    }

    /// Get a human-readable label for this driver mode
    pub fn label(&self) -> &'static str {
        match self {
            CdTextDriver::Disabled => "Disabled",
            CdTextDriver::Auto => "Auto",
            CdTextDriver::GenericMmcRaw => "Raw Mode",
            CdTextDriver::GenericMmcSubchannel => "Sub-channel Mode",
            CdTextDriver::GenericMmcCueSheet => "CUE Sheet Mode",
        }
    }

    /// Get a description for this driver mode
    pub fn description(&self) -> &'static str {
        match self {
            CdTextDriver::Disabled => "No CD-TEXT metadata",
            CdTextDriver::Auto => "Let cdrdao auto-detect driver",
            CdTextDriver::GenericMmcRaw => "generic-mmc-raw (most compatible)",
            CdTextDriver::GenericMmcSubchannel => "generic-mmc:0x10 (force sub-channel)",
            CdTextDriver::GenericMmcCueSheet => "generic-mmc:0x20000 (Pioneer drives)",
        }
    }

    /// Create from index (for UI dropdown)
    pub fn from_index(index: i32) -> Self {
        match index {
            0 => CdTextDriver::Auto,
            1 => CdTextDriver::GenericMmcRaw,
            2 => CdTextDriver::GenericMmcSubchannel,
            3 => CdTextDriver::GenericMmcCueSheet,
            _ => CdTextDriver::Auto,
        }
    }
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

    // cdrdao outputs drive info to stderr, not stdout (quirk of the tool)
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stdout, stderr);
    let mut drives = Vec::new();

    // Parse scanbus output
    // Linux format: X,Y,Z : Vendor Model
    // macOS format: IOService:/long/path : VENDOR, Model, Version
    for line in combined.lines() {
        // Find the last " : " which separates device from info
        if let Some(pos) = line.rfind(" : ") {
            let device = line[..pos].trim().to_string();
            let rest = line[pos + 3..].trim();

            // Check if it's macOS format (comma-separated: VENDOR, Model, Version)
            if rest.contains(',') {
                let parts: Vec<&str> = rest.splitn(3, ',').collect();
                if parts.len() >= 2 {
                    drives.push(CdDrive {
                        device,
                        vendor: parts[0].trim().to_string(),
                        model: parts[1].trim().to_string(),
                    });
                }
            } else {
                // Linux format (space-separated: Vendor Model)
                let parts: Vec<&str> = rest.splitn(2, ' ').collect();
                if parts.len() >= 2 {
                    drives.push(CdDrive {
                        device,
                        vendor: parts[0].to_string(),
                        model: parts[1].to_string(),
                    });
                }
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
    /// CD-TEXT driver mode
    pub cd_text_driver: CdTextDriver,
}

impl Default for BurnOptions {
    fn default() -> Self {
        Self {
            device: "/dev/sr0".to_string(),
            speed: 0,
            simulate: false,
            eject: true,
            cd_text_driver: CdTextDriver::Auto,
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
        if let Some((flag, value)) = self.options.cd_text_driver.driver_args() {
            args.push(flag);
            args.push(value);
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

        // Print command for debugging
        eprintln!("[cdrdao] Running: cdrdao {}", args.join(" "));

        let output = cmd
            .args(&args)
            .output()
            .map_err(|e| CdrdaoError::ExecutionError(e.to_string()))?;

        // Print all output to console for debugging
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !stdout.is_empty() {
            eprintln!("[cdrdao stdout]\n{}", stdout);
        }
        if !stderr.is_empty() {
            eprintln!("[cdrdao stderr]\n{}", stderr);
        }
        eprintln!("[cdrdao] Exit status: {}", output.status);

        if !output.status.success() {
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
        assert_eq!(options.cd_text_driver, CdTextDriver::Auto);
        assert!(!options.simulate);
    }

    #[test]
    fn test_cd_text_driver_args() {
        assert_eq!(CdTextDriver::Disabled.driver_args(), None);
        assert_eq!(CdTextDriver::Auto.driver_args(), None);
        assert_eq!(
            CdTextDriver::GenericMmcRaw.driver_args(),
            Some(("--driver", "generic-mmc-raw"))
        );
        assert_eq!(
            CdTextDriver::GenericMmcSubchannel.driver_args(),
            Some(("--driver", "generic-mmc:0x10"))
        );
        assert_eq!(
            CdTextDriver::GenericMmcCueSheet.driver_args(),
            Some(("--driver", "generic-mmc:0x20000"))
        );
    }

    #[test]
    fn test_cd_text_driver_from_index() {
        assert_eq!(CdTextDriver::from_index(0), CdTextDriver::Auto);
        assert_eq!(CdTextDriver::from_index(1), CdTextDriver::GenericMmcRaw);
        assert_eq!(CdTextDriver::from_index(2), CdTextDriver::GenericMmcSubchannel);
        assert_eq!(CdTextDriver::from_index(3), CdTextDriver::GenericMmcCueSheet);
        assert_eq!(CdTextDriver::from_index(99), CdTextDriver::Auto); // Invalid defaults to Auto
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

    #[test]
    fn test_list_drives() {
        // This test verifies list_drives works (if cdrdao is installed)
        if !is_available() {
            return;
        }
        match list_drives() {
            Ok(drives) => {
                println!("Found {} drives", drives.len());
                for d in &drives {
                    println!("  Drive: {} {} ({})", d.vendor, d.model, d.device);
                }
            }
            Err(e) => {
                println!("Error listing drives: {}", e);
            }
        }
    }

    #[test]
    fn test_parse_scanbus_macos() {
        // Test parsing macOS cdrdao scanbus output
        let macos_output = "IOService:/AppleARMPE/arm-io@10F00000/AppleH16GFamilyIO/usb-drd2@92280000/AppleT8132USBXHCI@02000000/usb-drd2-port-hs@02100000/Pioneer Blu-ray Drive@02100000/IOUSBHostInterface@0/IOUSBMassStorageInterfaceNub/IOUSBMassStorageDriverNub/IOUSBMassStorageDriver/IOSCSILogicalUnitNub@0/IOSCSIPeripheralDeviceType05/IOBDServices : PIONEER, BD-RW   BDR-XD07, 1.03";

        // Simulate parsing logic
        let mut drives = Vec::new();
        for line in macos_output.lines() {
            if let Some(pos) = line.rfind(" : ") {
                let device = line[..pos].trim().to_string();
                let rest = line[pos + 3..].trim();
                if rest.contains(',') {
                    let parts: Vec<&str> = rest.splitn(3, ',').collect();
                    if parts.len() >= 2 {
                        drives.push(CdDrive {
                            device,
                            vendor: parts[0].trim().to_string(),
                            model: parts[1].trim().to_string(),
                        });
                    }
                }
            }
        }

        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].vendor, "PIONEER");
        assert_eq!(drives[0].model, "BD-RW   BDR-XD07");
        assert!(drives[0].device.contains("IOBDServices"));
    }

    #[test]
    fn test_parse_scanbus_linux() {
        // Test parsing Linux cdrdao scanbus output
        let linux_output = "0,0,0 : SONY, CD-RW  CRX230E, 1.1";

        let mut drives = Vec::new();
        for line in linux_output.lines() {
            if let Some(pos) = line.rfind(" : ") {
                let device = line[..pos].trim().to_string();
                let rest = line[pos + 3..].trim();
                if rest.contains(',') {
                    let parts: Vec<&str> = rest.splitn(3, ',').collect();
                    if parts.len() >= 2 {
                        drives.push(CdDrive {
                            device,
                            vendor: parts[0].trim().to_string(),
                            model: parts[1].trim().to_string(),
                        });
                    }
                }
            }
        }

        assert_eq!(drives.len(), 1);
        assert_eq!(drives[0].device, "0,0,0");
        assert_eq!(drives[0].vendor, "SONY");
        assert_eq!(drives[0].model, "CD-RW  CRX230E");
    }
}
