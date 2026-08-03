use std::sync::Arc;

use buzz_runtime::{
    read_bounded_frame, AuthorizedCapability, ControlError, ControlHandlerFn, ControlOperation,
    ControlPayload, ControlRequest, ControlServerConfig, RuntimeServer, SecretToken,
    CONTROL_PROTOCOL_VERSION, MAX_CONTROL_REQUEST_BYTES,
};
use tokio::io::{AsyncWriteExt, DuplexStream};
use uuid::Uuid;

async fn response_for(
    config: ControlServerConfig,
    request: ControlRequest,
) -> buzz_runtime::ControlResponse {
    let server = RuntimeServer::bind(config.clone()).await.unwrap();
    let address = server.local_addr().unwrap();
    tokio::spawn(server.serve(Arc::new(ControlHandlerFn(
        |_capability: AuthorizedCapability, _operation: ControlOperation| async {
            Ok::<_, ControlError>(ControlPayload::Ack)
        },
    ))));
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let bytes = serde_json::to_vec(&request).unwrap();
    stream
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .unwrap();
    stream.write_all(&bytes).await.unwrap();
    let response =
        buzz_runtime::read_bounded_frame(&mut stream, buzz_runtime::MAX_CONTROL_RESPONSE_BYTES)
            .await
            .unwrap();
    serde_json::from_slice(&response).unwrap()
}

#[tokio::test]
async fn oversized_request_is_rejected_from_header_before_payload_allocation() {
    let (mut writer, mut reader): (DuplexStream, DuplexStream) = tokio::io::duplex(8);
    writer
        .write_all(&((MAX_CONTROL_REQUEST_BYTES as u32) + 1).to_be_bytes())
        .await
        .unwrap();
    let error = read_bounded_frame(&mut reader, MAX_CONTROL_REQUEST_BYTES)
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        buzz_runtime::ServerError::FrameTooLarge { .. }
    ));
}

#[tokio::test]
async fn wrong_generation_and_token_have_same_generic_error() {
    let generation = Uuid::new_v4();
    let config = ControlServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        runtime_id: "runtime".into(),
        generation,
        controller_token: SecretToken::new("controller"),
        model_token: SecretToken::new("model"),
    };
    let wrong_generation = response_for(
        config.clone(),
        ControlRequest {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            generation: Uuid::new_v4(),
            control_token: config.controller_token.clone(),
            operation: ControlOperation::Hello,
        },
    )
    .await;
    let wrong_token = response_for(
        config,
        ControlRequest {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            generation,
            control_token: SecretToken::new("wrong"),
            operation: ControlOperation::Hello,
        },
    )
    .await;
    assert_eq!(wrong_generation.error, Some(ControlError::unauthorized()));
    assert_eq!(wrong_token.error, Some(ControlError::unauthorized()));
}

#[tokio::test]
async fn model_capability_cannot_shutdown() {
    let generation = Uuid::new_v4();
    let config = ControlServerConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        runtime_id: "runtime".into(),
        generation,
        controller_token: SecretToken::new("controller"),
        model_token: SecretToken::new("model"),
    };
    let response = response_for(
        config.clone(),
        ControlRequest {
            protocol_version: CONTROL_PROTOCOL_VERSION,
            generation,
            control_token: config.model_token,
            operation: ControlOperation::Shutdown,
        },
    )
    .await;
    assert_eq!(response.error, Some(ControlError::unauthorized()));
}
