/// Integration tests that verify ureq can reach the LRCLIB API.
/// Run with: cargo test -- --ignored

fn build_agent() -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(10));

    // Use system certificate store (needed for TLS-intercepting proxies).
    let mut root_store = rustls::RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        let _ = root_store.add(cert);
    }
    if !root_store.is_empty() {
        let tls_config = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("ring provider supports default TLS versions")
        .with_root_certificates(root_store)
        .with_no_client_auth();
        builder = builder.tls_config(std::sync::Arc::new(tls_config));
    }

    if let Ok(url) = std::env::var("https_proxy").or_else(|_| std::env::var("HTTPS_PROXY")) {
        if let Ok(proxy) = ureq::Proxy::new(&url) {
            builder = builder.proxy(proxy);
        }
    }

    builder.build()
}

#[test]
#[ignore] // requires network
fn lrclib_exact_match_reachable() {
    let agent = build_agent();
    let resp = agent
        .get("https://lrclib.net/api/get")
        .set("User-Agent", "cue-test/0.1.0")
        .query("track_name", "Bohemian Rhapsody")
        .query("artist_name", "Queen")
        .query("album_name", "A Night at the Opera")
        .query("duration", "354")
        .call();

    match resp {
        Ok(r) => {
            assert_eq!(r.status(), 200, "Expected 200");
            let body: serde_json::Value = r.into_json().expect("failed to parse JSON");
            assert!(
                body.get("syncedLyrics").is_some() || body.get("plainLyrics").is_some(),
                "Response missing lyrics fields: {body}"
            );
        }
        Err(e) => {
            panic!("ureq exact match call failed: {e}");
        }
    }
}

#[test]
#[ignore] // requires network
fn lrclib_search_reachable() {
    let agent = build_agent();
    let resp = agent
        .get("https://lrclib.net/api/search")
        .set("User-Agent", "cue-test/0.1.0")
        .query("track_name", "Bohemian Rhapsody")
        .query("artist_name", "Queen")
        .call();

    match resp {
        Ok(r) => {
            assert_eq!(r.status(), 200);
            let body: Vec<serde_json::Value> = r.into_json().expect("failed to parse JSON");
            assert!(!body.is_empty(), "Search returned empty results");
        }
        Err(e) => {
            panic!("ureq search call failed: {e}");
        }
    }
}
