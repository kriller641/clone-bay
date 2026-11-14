//! Where releases are fetched from: GitHub, or a folder on this PC for
//! trying an update before a release depends on it.

use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use crate::update::{Offer, ReleaseSource, UpdateError};

const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// For the two small text files.
const TEXT_TIMEOUT: Duration = Duration::from_secs(20);
/// For the exe, on a slow connection.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);
/// Far more than a checksum file or a signature holds.
const MAX_TEXT_BYTES: u64 = 64 * 1024;

/// The page a person downloads the newest release from.
pub fn download_page() -> String {
    format!("{REPOSITORY}/releases/latest")
}

fn latest_url(name: &str) -> String {
    format!("{REPOSITORY}/releases/latest/download/{name}")
}

fn asset_url(offer: &Offer) -> String {
    format!(
        "{REPOSITORY}/releases/download/v{}/{}",
        offer.version, offer.asset
    )
}

fn network(error: impl std::fmt::Display) -> UpdateError {
    UpdateError::Network(error.to_string())
}

/// The project's releases on GitHub, through its plain download links.
pub struct GitHubReleases {
    text: ureq::Agent,
    download: ureq::Agent,
}

impl GitHubReleases {
    pub fn new(app_version: &str) -> Self {
        Self {
            text: crate::http::agent(app_version, TEXT_TIMEOUT, true),
            download: crate::http::agent(app_version, DOWNLOAD_TIMEOUT, true),
        }
    }
}

impl ReleaseSource for GitHubReleases {
    fn latest_text(&self, name: &str) -> Result<String, UpdateError> {
        let mut response = self.text.get(latest_url(name)).call().map_err(network)?;
        response
            .body_mut()
            .with_config()
            .limit(MAX_TEXT_BYTES)
            .read_to_string()
            .map_err(network)
    }

    fn open(&self, offer: &Offer) -> Result<Box<dyn Read>, UpdateError> {
        let response = self
            .download
            .get(asset_url(offer))
            .call()
            .map_err(network)?;
        Ok(Box::new(response.into_body().into_reader()))
    }
}

/// A release laid out in a folder: the listing, its signature and the exe
/// side by side.
pub struct FolderSource {
    folder: PathBuf,
}

impl FolderSource {
    pub fn new(folder: PathBuf) -> Self {
        Self { folder }
    }
}

impl ReleaseSource for FolderSource {
    fn latest_text(&self, name: &str) -> Result<String, UpdateError> {
        fs::read_to_string(self.folder.join(name)).map_err(network)
    }

    fn open(&self, offer: &Offer) -> Result<Box<dyn Read>, UpdateError> {
        let file = fs::File::open(self.folder.join(&offer.asset)).map_err(network)?;
        Ok(Box::new(file))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;
    use crate::update::{Offer, ReleaseSource, UpdateError, Version, LISTING_NAME};

    fn offer(asset: &str) -> Offer {
        Offer {
            version: Version::parse("0.1.5").unwrap(),
            asset: asset.to_owned(),
            sha256: [0; 32],
        }
    }

    #[test]
    fn a_folder_serves_the_listing_and_the_exe() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LISTING_NAME), "listing").unwrap();
        std::fs::write(dir.path().join("clone-bay-v0.1.5-windows-x64.exe"), b"exe").unwrap();
        let source = FolderSource::new(dir.path().to_path_buf());
        assert_eq!(source.latest_text(LISTING_NAME).unwrap(), "listing");
        let mut body = Vec::new();
        source
            .open(&offer("clone-bay-v0.1.5-windows-x64.exe"))
            .unwrap()
            .read_to_end(&mut body)
            .unwrap();
        assert_eq!(body, b"exe");
    }

    #[test]
    fn a_folder_without_the_file_is_a_network_error() {
        let dir = tempfile::tempdir().unwrap();
        let source = FolderSource::new(dir.path().to_path_buf());
        assert!(matches!(
            source.latest_text(LISTING_NAME),
            Err(UpdateError::Network(_))
        ));
        assert!(matches!(
            source.open(&offer("missing.exe")),
            Err(UpdateError::Network(_))
        ));
    }

    #[test]
    fn the_links_point_at_the_projects_releases() {
        let repo = "https://github.com/kriller641/clone-bay";
        assert_eq!(download_page(), format!("{repo}/releases/latest"));
        assert_eq!(
            latest_url("SHA256SUMS.txt"),
            format!("{repo}/releases/latest/download/SHA256SUMS.txt")
        );
        assert_eq!(
            asset_url(&offer("clone-bay-v0.1.5-windows-x64.exe")),
            format!("{repo}/releases/download/v0.1.5/clone-bay-v0.1.5-windows-x64.exe")
        );
    }

    /// Makes one real request to GitHub. Run it by hand:
    /// `cargo test -p clone-bay-core --lib -- --ignored update_live`
    #[test]
    #[ignore = "talks to GitHub"]
    fn update_live_the_latest_release_lists_a_windows_download() {
        let source = GitHubReleases::new(env!("CARGO_PKG_VERSION"));
        let listing = source.latest_text(LISTING_NAME).unwrap();
        crate::update::parse_listing(&listing).unwrap();
    }
}
