use super::websocket_startup_banner;
use crate::transport::auth::AppServerWebsocketAuthSettings;
use crate::transport::auth::policy_from_settings;

#[test]
fn websocket_startup_banner_includes_generated_token() {
    let policy = policy_from_settings(&AppServerWebsocketAuthSettings::default())
        .expect("generated token policy should build");
    let token = policy
        .generated_query_token()
        .expect("generated policy should expose its token");
    let banner = websocket_startup_banner("127.0.0.1:4500".parse().unwrap(), &policy)
        .replace(token, "<generated-token>");

    insta::assert_snapshot!(banner);
}
