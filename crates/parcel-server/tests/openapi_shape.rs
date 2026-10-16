//! The OpenAPI document is pinned and committed.

use parcel_server::openapi;
use stridelabs_http::openapi::{assert_committed_spec_is_fresh, documented_pairs, expected_pairs};

/// The route-pinning test: an exact (method, path) snapshot. A route added
/// with `.route` instead of `.routes(routes!(..))`, or an annotation that
/// should not exist, changes this set and fails here.
#[test]
fn the_documented_path_method_set_is_exact() {
    assert_eq!(
        documented_pairs(&openapi::spec()),
        expected_pairs(&[
            ("GET", "/api/v1/info"),
            ("GET", "/api/v1/me"),
            ("GET", "/api/v1/parcels"),
            ("POST", "/api/v1/parcels"),
            ("GET", "/api/v1/parcels/{id}"),
            ("PATCH", "/api/v1/parcels/{id}"),
            ("DELETE", "/api/v1/parcels/{id}"),
            ("GET", "/api/v1/parcels/{id}/view"),
            ("GET", "/api/v1/parcels/{id}/versions"),
            ("POST", "/api/v1/parcels/{id}/versions"),
            ("GET", "/api/v1/parcels/{id}/versions/{version}"),
            ("POST", "/api/v1/parcels/{id}/versions/{version}/restore"),
            (
                "GET",
                "/api/v1/parcels/{id}/versions/{version}/files/{path}"
            ),
            ("GET", "/api/v1/tokens"),
            ("POST", "/api/v1/tokens"),
            ("DELETE", "/api/v1/tokens/current"),
            ("DELETE", "/api/v1/tokens/{id}"),
            ("POST", "/api/v1/cli/device"),
            ("POST", "/api/v1/cli/device/token"),
            ("GET", "/api/v1/cli/device/{user_code}"),
            ("POST", "/api/v1/cli/device/{user_code}/approve"),
            ("POST", "/api/v1/cli/device/{user_code}/deny"),
        ]),
    );
}

/// `openapi.json` must be exactly what `parcel-server openapi` prints today.
#[test]
fn the_committed_openapi_json_matches_a_fresh_export() {
    assert_committed_spec_is_fresh(
        concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json"),
        &openapi::spec(),
        "cargo run -p parcel-server -- openapi > crates/parcel-server/openapi.json",
    );
}
