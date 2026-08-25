use axum::{routing::get, Json, Router};
use business_auth_gateway::{auth::JwtVerifier, Config};
use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde::Serialize;
use serde_json::json;
use std::{collections::HashSet, time::Duration};

const RSA_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC59n6mcpZnaZZk
fTKaDXFsoBtQS8YPg9jSBI003QhZ7ktTALCpVQ70vcNADM51S8f+Hf1Ob8gNLufC
Cmw2bIzIsfuZOruaaEBu1JxxqU2jQJ97SQVYNeZOSRmgCEFxIHz5dyTNw78u+eY5
SnmYn1fK1AIkfibJMikKdET9jmh8Y2tf+9uM0s+9u2LDooh2Of8tyLl/LftuHXzr
uLYUfVFC1boZFmk96Elmwnrpm6NzP+gLhpGkWn7VYJ6HXuDlrPRoma9em3z5XMVk
HFwm/PNEj8/m0ljt1/MJJ38zxOXfkNuyGvu4aBl7W7bicrgE8A4gLWdGr98cyXvp
UBcincFXAgMBAAECggEADNlKBUB2lueV6L85Gf7NdEwYXF3CmLuAkCZDXBXc6AXN
y4ref0dYFLNF2deTWPRm+ZkN3E0J6ACdKmw474ZkMHu9CWh5ojYLeaeUCvW/3lnu
iwClGg6t/oEQ1bIBqjTXSGSh34ZjtSBhdq+Szjz4PmnpSfyIu9m3jzgwX3U8T+0a
b6wOj8qKcOk0N/J6lOg2BsPnQW/p9m/xpS8eIGgHCIkGleUdg5nL4g5iVq6OXm3X
knmwKEVII84lURrd79V+jTeeB6VxIRWAArxN/DgulGJkXRAlWypJ5T3S6rDLvZ9u
7kvytovVw0nPYKQG9A5NqYIr28HmQqx40UJDFmT+oQKBgQDkhalcUs2hrQ3mKfCm
ARUnrWz4J4hwCfBP/3YBGRzfW35NraKmLglF7ooP1QMJQMMbKZLDlVP3OOZCVR+S
2h35nVG3XVswd+jnnnGARleELgqOJN98vZTqXKfyP97cvsPgMV2G9/h71FiKHHwX
nUZj5Uts0gkMqiEfhh35ZvXZoQKBgQDQUsepee/L7E7bzmSn+BmkZ42NT5ca/6mY
UDftzFvxw3UPVHMu0wpu8QyD7uvk9zLX0EoPeNf6a2YashmrRPuvI2QNffzkvlyd
mdqSRuib4PlbOqtUO4N4kHkFcmcLBGkI75Ch9hDwnE3noj0XmEvvuOFkam64Luf5
A55q7Dln9wKBgCM3xiYITNCBzwaNqBytRglbXNPRo+FAZtytTg5VRHHXs9tcyxg5
OAyi+nv+I/2lEWx6N7gUp2AOUM4gOEF1g/EYIaPUq10I3cf0TyGptYsVXWMSo66h
uPV1WhynYz052Q4QDY3jYVQUIaEHSsiI4HQ8vicDJ4ngHkKxdKUfDPyBAoGADNhd
2VRcdd1/S0xhpn3Ezv9XmhQDRDXpdivUFwSX0sNzj1tsssFujkKsu+Hah8a6StZc
CrIv1xASPqkmrgnV3wm2nKJdGpmmSk13TbezlhD8LyTh9ZKp26BE5hIUyngeJd/n
siTjDIMGxraZP8AzRnfG5hMt+oth4FfZx8wDCicCgYEAtRFkQatBQqtHALJV+vgX
4D9RI88Opf111gxsKBBwDKLGI9jM7+P9Prsh5khzS86K28SemTBzq2OQFICxLdk5
bL6cOgcBw5+q33Qoc+ZV7fwZU3MOsvP/DSLzhi8OrUyOAbgneG1EpniX8FgkW6Uy
ocwAiKOnjorUn5QaySglJ38=
-----END PRIVATE KEY-----"#;

const MODULUS: &str = "ufZ-pnKWZ2mWZH0ymg1xbKAbUEvGD4PY0gSNNN0IWe5LUwCwqVUO9L3DQAzOdUvH_h39Tm_IDS7nwgpsNmyMyLH7mTq7mmhAbtSccalNo0Cfe0kFWDXmTkkZoAhBcSB8-XckzcO_LvnmOUp5mJ9XytQCJH4myTIpCnRE_Y5ofGNrX_vbjNLPvbtiw6KIdjn_Lci5fy37bh1867i2FH1RQtW6GRZpPehJZsJ66Zujcz_oC4aRpFp-1WCeh17g5az0aJmvXpt8-VzFZBxcJvzzRI_P5tJY7dfzCSd_M8Tl35Dbshr7uGgZe1u24nK4BPAOIC1nRq_fHMl76VAXIp3BVw";

#[derive(Clone, Serialize)]
struct TestClaims {
    iss: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub: Option<String>,
    exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    aud: Option<String>,
    azp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    events: Option<serde_json::Value>,
}

fn config(issuer: String) -> Config {
    Config {
        database_url: "postgres://unused".into(),
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        authentik_issuer: issuer,
        workbench_client_id: "workbench".into(),
        business_client_id: "business".into(),
        allowed_workbench_origins: HashSet::from(["tauri://localhost".into()]),
        business_origin: "https://business.test".into(),
        challenge_ttl: Duration::from_secs(90),
        embed_ttl: Duration::from_secs(30),
        business_ttl: Duration::from_secs(3600),
        rate_limit: 10,
        cleanup_interval: Duration::from_secs(60),
        cookie_name: "__Host-test".into(),
        cookie_secure: true,
        deployment_id: "test".into(),
        global_logout_redirect_uri: "https://workbench.test/".into(),
        business_agent_read_enabled: false,
        business_read_mcp_audience: "business-read-mcp".into(),
        agent_delegation_ttl: Duration::from_secs(300),
        agent_delegation_max_calls: 20,
        business_agent_rate_limit_per_minute: 10,
        business_read_service_credential: None,
    }
}

fn token(claims: &TestClaims) -> String {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key".into());
    encode(
        &header,
        claims,
        &EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY.as_bytes()).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn validates_signature_issuer_audience_client_expiry_and_subject() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer = format!("http://{}", listener.local_addr().unwrap());
    let jwks_uri = format!("{issuer}/jwks");
    let app = Router::new()
        .route(
            "/.well-known/openid-configuration",
            get(move || {
                let jwks_uri = jwks_uri.clone();
                async move { Json(json!({"jwks_uri": jwks_uri})) }
            }),
        )
        .route(
            "/jwks",
            get(|| async {
                Json(json!({"keys":[{
                    "kty":"RSA", "use":"sig", "kid":"test-key", "alg":"RS256",
                    "n":MODULUS, "e":"AQAB"
                }]}))
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let verifier = JwtVerifier::new(&config(issuer.clone()));
    let valid = TestClaims {
        iss: issuer.clone(),
        sub: Some("enterprise-user".into()),
        exp: Utc::now().timestamp() + 300,
        aud: Some("workbench".into()),
        azp: "workbench".into(),
        sid: None,
        events: None,
    };
    assert!(verifier.workbench(&token(&valid)).await.is_ok());
    assert!(verifier
        .workbench(&token(&TestClaims {
            aud: Some("authentik-api".into()),
            ..valid.clone()
        }))
        .await
        .is_ok());
    assert!(verifier
        .workbench(&token(&TestClaims {
            aud: None,
            ..valid.clone()
        }))
        .await
        .is_ok());

    let mut bad_signature = token(&valid);
    let final_byte = bad_signature.pop().unwrap();
    bad_signature.push(if final_byte == 'A' { 'B' } else { 'A' });
    assert!(verifier.workbench(&bad_signature).await.is_err());

    for invalid in [
        TestClaims {
            iss: "https://wrong-issuer.test".into(),
            ..valid.clone()
        },
        TestClaims {
            aud: Some("wrong-audience".into()),
            azp: "wrong-client".into(),
            ..valid.clone()
        },
        TestClaims {
            azp: "wrong-client".into(),
            ..valid.clone()
        },
        TestClaims {
            exp: Utc::now().timestamp() - 60,
            ..valid.clone()
        },
        TestClaims {
            sub: None,
            ..valid.clone()
        },
    ] {
        assert!(verifier.workbench(&token(&invalid)).await.is_err());
    }

    let logout = TestClaims {
        aud: Some("business".into()),
        azp: "business".into(),
        sid: Some("oidc-session".into()),
        events: Some(json!({
            "http://schemas.openid.net/event/backchannel-logout": {}
        })),
        ..valid
    };
    assert!(verifier.logout(&token(&logout)).await.is_ok());
    assert!(verifier
        .logout(&token(&TestClaims {
            sid: None,
            ..logout.clone()
        }))
        .await
        .is_ok());
    assert!(verifier
        .logout(&token(&TestClaims {
            events: None,
            ..logout
        }))
        .await
        .is_err());
}
