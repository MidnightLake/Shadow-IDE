use base64::Engine;
use rcgen::{CertificateParams, DnType, KeyPair};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

pub const REMOTE_PERMISSION_KEYS: &[&str] =
    &["filesystem", "terminal", "llm", "workspace", "agent"];

pub fn default_remote_permissions() -> Vec<String> {
    REMOTE_PERMISSION_KEYS
        .iter()
        .map(|permission| (*permission).to_string())
        .collect()
}

fn normalize_permissions(permissions: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::new();
    for permission in permissions {
        if !REMOTE_PERMISSION_KEYS.contains(&permission.as_str()) {
            return Err(format!("Unsupported remote permission: {}", permission));
        }
        if !normalized.contains(&permission) {
            normalized.push(permission);
        }
    }
    if normalized.is_empty() {
        return Err("At least one remote permission must be granted".to_string());
    }
    Ok(normalized)
}

/// Stores generated server certs and paired client fingerprints.
pub struct PairingManager {
    pub data_dir: PathBuf,
    pub server_cert_pem: Mutex<Option<String>>,
    pub server_key_pem: Mutex<Option<String>>,
    pub paired_devices: Mutex<HashMap<String, PairedDevice>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    pub id: String,
    pub name: String,
    pub fingerprint: String,
    pub paired_at: String,
    #[serde(default = "default_remote_permissions")]
    pub permissions: Vec<String>,
}

/// Data encoded into the QR code for pairing.
#[derive(Debug, Serialize, Deserialize)]
pub struct PairingPayload {
    pub host: String,
    pub port: u16,
    pub server_fingerprint: String,
    pub pairing_token: String,
}

impl PairingManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&data_dir);

        let manager = Self {
            data_dir: data_dir.clone(),
            server_cert_pem: Mutex::new(None),
            server_key_pem: Mutex::new(None),
            paired_devices: Mutex::new(HashMap::new()),
        };

        // Try loading existing certs
        manager.load_certs();
        manager.load_paired_devices();

        manager
    }

    fn cert_path(&self) -> PathBuf {
        self.data_dir.join("server_cert.pem")
    }

    fn key_path(&self) -> PathBuf {
        self.data_dir.join("server_key.pem")
    }

    fn devices_path(&self) -> PathBuf {
        self.data_dir.join("paired_devices.json")
    }

    fn load_certs(&self) {
        if let (Ok(cert), Ok(key)) = (
            fs::read_to_string(self.cert_path()),
            fs::read_to_string(self.key_path()),
        ) {
            if let Ok(mut guard) = self.server_cert_pem.lock() {
                *guard = Some(cert);
            }
            if let Ok(mut guard) = self.server_key_pem.lock() {
                *guard = Some(key);
            }
        }
    }

    fn load_paired_devices(&self) {
        if let Ok(data) = fs::read_to_string(self.devices_path()) {
            if let Ok(devices) = serde_json::from_str::<HashMap<String, PairedDevice>>(&data) {
                if let Ok(mut guard) = self.paired_devices.lock() {
                    *guard = devices;
                }
            }
        }
    }

    fn save_paired_devices(&self) -> Result<(), String> {
        let devices = self
            .paired_devices
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let json = serde_json::to_string_pretty(&*devices)
            .map_err(|e| format!("Serialize error: {}", e))?;
        fs::write(self.devices_path(), json).map_err(|e| format!("Write error: {}", e))
    }

    /// Generate a new self-signed server certificate.
    pub fn generate_server_cert(&self) -> Result<(), String> {
        let key_pair = KeyPair::generate().map_err(|e| format!("Key generation failed: {}", e))?;

        let mut params = CertificateParams::default();
        params
            .distinguished_name
            .push(DnType::CommonName, "ShadowIDE Server");
        params
            .distinguished_name
            .push(DnType::OrganizationName, "ShadowIDE");

        // Valid for 10 years
        let now = rcgen::date_time_ymd(2024, 1, 1);
        let future = rcgen::date_time_ymd(2034, 1, 1);
        params.not_before = now;
        params.not_after = future;

        // Add SANs for local access
        params.subject_alt_names = vec![
            rcgen::SanType::DnsName("localhost".try_into().map_err(|e| format!("{}", e))?),
            rcgen::SanType::IpAddress(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)),
        ];

        // Add local IP if available
        if let Ok(ip) = local_ip_address::local_ip() {
            params.subject_alt_names.push(rcgen::SanType::IpAddress(ip));
        }

        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| format!("Cert signing failed: {}", e))?;

        let cert_pem = cert.pem();
        let key_pem = key_pair.serialize_pem();

        // Save to disk
        fs::write(self.cert_path(), &cert_pem)
            .map_err(|e| format!("Failed to write cert: {}", e))?;
        fs::write(self.key_path(), &key_pem).map_err(|e| format!("Failed to write key: {}", e))?;

        if let Ok(mut guard) = self.server_cert_pem.lock() {
            *guard = Some(cert_pem);
        }
        if let Ok(mut guard) = self.server_key_pem.lock() {
            *guard = Some(key_pem);
        }

        Ok(())
    }

    /// Get a simple SHA-256 fingerprint of the server cert.
    pub fn server_fingerprint(&self) -> Result<String, String> {
        let cert_pem = self
            .server_cert_pem
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let cert_pem = cert_pem.as_ref().ok_or("No server certificate generated")?;

        // Parse PEM and hash the DER
        let mut reader = std::io::BufReader::new(cert_pem.as_bytes());
        let certs = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("PEM parse error: {}", e))?;

        let cert_der = certs.first().ok_or("No certificate found in PEM")?;

        // SHA-256 fingerprint
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(cert_der.as_ref());
        let fingerprint = hash
            .iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(":");
        Ok(fingerprint)
    }

    /// Generate a pairing payload for a QR code.
    pub fn generate_pairing_data(&self, port: u16) -> Result<PairingPayload, String> {
        let fingerprint = self.server_fingerprint()?;

        let host = local_ip_address::local_ip()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|_| "localhost".to_string());

        let token = uuid::Uuid::new_v4().to_string();

        Ok(PairingPayload {
            host,
            port,
            server_fingerprint: fingerprint,
            pairing_token: token,
        })
    }

    /// Generate a QR code as a string of the pairing data (returns the data URL).
    pub fn generate_qr_code(&self, port: u16) -> Result<(String, String), String> {
        let payload = self.generate_pairing_data(port)?;
        let json = serde_json::to_string(&payload).map_err(|e| format!("JSON error: {}", e))?;

        let code = qrcode::QrCode::new(json.as_bytes())
            .map_err(|e| format!("QR generation failed: {}", e))?;

        // Render as SVG
        let svg = code
            .render::<qrcode::render::svg::Color>()
            .min_dimensions(200, 200)
            .quiet_zone(true)
            .build();

        // Base64 encode for inline display
        let b64 = base64::engine::general_purpose::STANDARD.encode(svg.as_bytes());
        let data_url = format!("data:image/svg+xml;base64,{}", b64);

        Ok((data_url, payload.pairing_token))
    }

    /// Hash a token/fingerprint with SHA-256 for secure storage.
    fn hash_token(token: &str) -> String {
        let hash = Sha256::digest(token.as_bytes());
        format!("{:x}", hash)
    }

    /// Register a new paired device. Stores a SHA-256 hash of the token, not the plaintext.
    pub fn add_paired_device(&self, name: String, token: String) -> Result<PairedDevice, String> {
        let hashed = Self::hash_token(&token);
        let device = PairedDevice {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            fingerprint: hashed,
            paired_at: chrono_now(),
            permissions: default_remote_permissions(),
        };

        self.paired_devices
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .insert(device.id.clone(), device.clone());

        self.save_paired_devices()?;
        Ok(device)
    }

    /// Verify a pairing token against stored hashed tokens. Returns true if any device matches.
    pub fn verify_device_token(&self, token: &str) -> bool {
        let hashed = Self::hash_token(token);
        self.paired_devices
            .lock()
            .map(|devices| devices.values().any(|d| d.fingerprint == hashed))
            .unwrap_or(false)
    }

    pub fn get_paired_device_by_token(&self, token: &str) -> Result<Option<PairedDevice>, String> {
        let hashed = Self::hash_token(token);
        let devices = self
            .paired_devices
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        Ok(devices
            .values()
            .find(|device| device.fingerprint == hashed)
            .cloned())
    }

    /// Remove a paired device.
    pub fn remove_paired_device(&self, id: &str) -> Result<(), String> {
        self.paired_devices
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .remove(id);

        self.save_paired_devices()
    }

    /// List all paired devices.
    pub fn list_paired_devices(&self) -> Result<Vec<PairedDevice>, String> {
        let devices = self
            .paired_devices
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        Ok(devices.values().cloned().collect())
    }

    pub fn update_device_permissions(
        &self,
        id: &str,
        permissions: Vec<String>,
    ) -> Result<PairedDevice, String> {
        let normalized = normalize_permissions(permissions)?;
        let updated = {
            let mut devices = self
                .paired_devices
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            let device = devices
                .get_mut(id)
                .ok_or_else(|| format!("Unknown paired device: {}", id))?;
            device.permissions = normalized;
            device.clone()
        };
        self.save_paired_devices()?;
        Ok(updated)
    }

    /// Check if a given certificate fingerprint is paired.
    #[allow(dead_code)]
    pub fn is_device_paired(&self, fingerprint: &str) -> bool {
        self.paired_devices
            .lock()
            .map(|devices| devices.values().any(|d| d.fingerprint == fingerprint))
            .unwrap_or(false)
    }
}

fn chrono_now() -> String {
    // Simple ISO-8601 timestamp without pulling in chrono
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", duration.as_secs())
}
