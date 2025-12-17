use serde::{Deserialize, Serialize};

/// CD-TEXT metadata for the album or tracks
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CdText {
    /// Title of the album or track
    pub title: Option<String>,
    /// Performer/artist
    pub performer: Option<String>,
    /// Songwriter
    pub songwriter: Option<String>,
    /// Composer
    pub composer: Option<String>,
    /// Arranger
    pub arranger: Option<String>,
    /// Message (additional text)
    pub message: Option<String>,
    /// Genre
    pub genre: Option<String>,
    /// Disc identification info
    pub disc_id: Option<String>,
}

impl CdText {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if any CD-TEXT data is present
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.performer.is_none()
            && self.songwriter.is_none()
            && self.composer.is_none()
            && self.arranger.is_none()
            && self.message.is_none()
            && self.genre.is_none()
            && self.disc_id.is_none()
    }
}

/// ISRC (International Standard Recording Code)
/// Format: CC-XXX-YY-NNNNN
/// - CC: Country code (2 letters)
/// - XXX: Registrant code (3 alphanumeric)
/// - YY: Year of reference (2 digits)
/// - NNNNN: Designation code (5 digits)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Isrc(String);

impl Isrc {
    /// Create a new ISRC code
    pub fn new(code: &str) -> Result<Self, String> {
        let isrc = Self(code.to_uppercase().replace('-', ""));
        isrc.validate()?;
        Ok(isrc)
    }

    /// Create ISRC from raw string without validation
    pub fn from_raw(code: String) -> Self {
        Self(code.to_uppercase().replace('-', ""))
    }

    /// Validate the ISRC format
    pub fn validate(&self) -> Result<(), String> {
        let code = &self.0;

        if code.len() != 12 {
            return Err(format!(
                "ISRC must be 12 characters, got {}",
                code.len()
            ));
        }

        // First 2 characters: country code (letters)
        if !code[0..2].chars().all(|c| c.is_ascii_alphabetic()) {
            return Err("Country code must be 2 letters".to_string());
        }

        // Characters 3-5: registrant code (alphanumeric)
        if !code[2..5].chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err("Registrant code must be 3 alphanumeric characters".to_string());
        }

        // Characters 6-7: year (digits)
        if !code[5..7].chars().all(|c| c.is_ascii_digit()) {
            return Err("Year must be 2 digits".to_string());
        }

        // Characters 8-12: designation code (digits)
        if !code[7..12].chars().all(|c| c.is_ascii_digit()) {
            return Err("Designation code must be 5 digits".to_string());
        }

        Ok(())
    }

    /// Get the ISRC as a string (no dashes)
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the ISRC formatted with dashes: CC-XXX-YY-NNNNN
    pub fn formatted(&self) -> String {
        format!(
            "{}-{}-{}-{}",
            &self.0[0..2],
            &self.0[2..5],
            &self.0[5..7],
            &self.0[7..12]
        )
    }

    /// Get the country code
    pub fn country(&self) -> &str {
        &self.0[0..2]
    }

    /// Get the registrant code
    pub fn registrant(&self) -> &str {
        &self.0[2..5]
    }

    /// Get the year
    pub fn year(&self) -> &str {
        &self.0[5..7]
    }

    /// Get the designation code
    pub fn designation(&self) -> &str {
        &self.0[7..12]
    }
}

impl std::fmt::Display for Isrc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.formatted())
    }
}

/// MCN (Media Catalog Number) / UPC/EAN
/// Format: 13 digits
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mcn(String);

impl Mcn {
    /// Create a new MCN
    pub fn new(code: &str) -> Result<Self, String> {
        let mcn = Self(code.replace(['-', ' '], ""));
        mcn.validate()?;
        Ok(mcn)
    }

    /// Create MCN from raw string without validation
    pub fn from_raw(code: String) -> Self {
        Self(code.replace(['-', ' '], ""))
    }

    /// Validate the MCN format
    pub fn validate(&self) -> Result<(), String> {
        if self.0.len() != 13 {
            return Err(format!("MCN must be 13 digits, got {}", self.0.len()));
        }

        if !self.0.chars().all(|c| c.is_ascii_digit()) {
            return Err("MCN must contain only digits".to_string());
        }

        // Validate check digit (EAN-13 algorithm)
        let digits: Vec<u32> = self.0.chars().map(|c| c.to_digit(10).unwrap()).collect();
        let mut sum = 0;
        for (i, &digit) in digits.iter().enumerate() {
            if i % 2 == 0 {
                sum += digit;
            } else {
                sum += digit * 3;
            }
        }

        if sum % 10 != 0 {
            return Err("Invalid MCN check digit".to_string());
        }

        Ok(())
    }

    /// Get the MCN as a string
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Mcn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_isrc_valid() {
        let isrc = Isrc::new("US-ABC-24-12345").unwrap();
        assert_eq!(isrc.country(), "US");
        assert_eq!(isrc.registrant(), "ABC");
        assert_eq!(isrc.year(), "24");
        assert_eq!(isrc.designation(), "12345");
        assert_eq!(isrc.formatted(), "US-ABC-24-12345");
    }

    #[test]
    fn test_isrc_invalid_length() {
        assert!(Isrc::new("US-ABC-24-1234").is_err());
    }

    #[test]
    fn test_isrc_invalid_country() {
        assert!(Isrc::new("12-ABC-24-12345").is_err());
    }

    #[test]
    fn test_mcn_valid() {
        // Example EAN-13: 5901234123457
        let mcn = Mcn::new("5901234123457").unwrap();
        assert_eq!(mcn.as_str(), "5901234123457");
    }

    #[test]
    fn test_mcn_invalid_check_digit() {
        assert!(Mcn::new("5901234123456").is_err());
    }

    #[test]
    fn test_cd_text_empty() {
        let cdtext = CdText::new();
        assert!(cdtext.is_empty());
    }
}
