use buzz_pair_relay_pkg::CONN_TIMEOUT;

use super::PAIRING_HARD_TIMEOUT;

#[test]
fn pair_relay_conn_timeout_outlives_desktop_pairing_hard_timeout() {
    assert!(
        CONN_TIMEOUT > PAIRING_HARD_TIMEOUT,
        "CONN_TIMEOUT ({CONN_TIMEOUT:?}) must exceed desktop PAIRING_HARD_TIMEOUT ({PAIRING_HARD_TIMEOUT:?})"
    );
}
