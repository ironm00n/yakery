use http::Uri;

/// Where an issuer's endpoints are, each an absolute URI: fixed, as [`crate::entra::endpoints`]
/// are, or named by its discovery.
#[derive(Clone, Debug)]
pub struct Endpoints {
    pub authorization: Uri,
    pub token: Uri,
    pub userinfo: Option<Uri>,
    pub device: Option<Uri>,
}
