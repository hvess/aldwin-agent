use crate::oauth::Authority;

/// A provider account that can be logged in to. A closed list, like the
/// provider catalogue in aldwin-llm: adding one means adding a variant and
/// the authority it names, and nothing else in this crate changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Account {
    /// x.ai, through a SuperGrok or X Premium subscription.
    Xai,
}

impl Account {
    /// The provider id `connections.yaml` keys this account by — the
    /// catalogue row's own.
    pub fn id(self) -> &'static str {
        match self {
            Account::Xai => "xai",
        }
    }

    /// The service, as a sentence names it.
    pub fn name(self) -> &'static str {
        match self {
            Account::Xai => "x.ai",
        }
    }

    /// What the account has to have for the provider to serve it — the
    /// fact beside the account in a list of connections.
    pub fn subscription(self) -> &'static str {
        match self {
            Account::Xai => "SuperGrok or X Premium",
        }
    }

    pub(crate) fn authority(self) -> Authority {
        match self {
            Account::Xai => Authority {
                device_url: XAI_DEVICE_URL.into(),
                token_url: XAI_TOKEN_URL.into(),
                client_id: XAI_CLIENT_ID.into(),
                scope: XAI_SCOPE.into(),
            },
        }
    }
}

// x.ai's authorization server, as its OIDC discovery document at
// `https://auth.x.ai/.well-known/openid-configuration` states it.
const XAI_DEVICE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";

/// The public client of xAI's own Grok Build CLI. It is a public client —
/// there is no secret to keep — and xAI offers no registration for a third
/// party's own; Zed, Warp, LiteLLM and the rest reuse this one. Recorded as
/// a decision in the spec rather than hidden, since it is the one thing
/// here that xAI could withdraw.
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// What the token has to be allowed to do: `grok-cli:access` and
/// `api:access` are the two the inference endpoint checks; `offline_access`
/// is what makes a refresh token come back at all.
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
