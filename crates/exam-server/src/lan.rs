use std::net::IpAddr;

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::AppState;

#[derive(Serialize)]
pub struct LanAddresses {
    pub addresses: Vec<String>,
    pub port: u16,
}

pub async fn lan_addresses(State(state): State<AppState>) -> Json<LanAddresses> {
    Json(LanAddresses {
        addresses: list_non_loopback_ipv4(),
        port: state.port,
    })
}

pub fn list_non_loopback_ipv4() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if let IpAddr::V4(v4) = iface.ip() {
                if !v4.is_loopback() {
                    let s = v4.to_string();
                    if !out.contains(&s) {
                        out.push(s);
                    }
                }
            }
        }
    }
    out.sort();
    out
}
