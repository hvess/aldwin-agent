use crate::oauth::Authority;

/// A provider account that can be logged in to. Adding one is a variant and
/// its `Authority`; nothing else in this crate changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Account {
    /// x.ai, through a SuperGrok or X Premium subscription.
    Xai,
}

impl Account {
    /// The key in `connections.yaml`; must equal the aldwin-llm catalogue's
    /// provider id.
    pub fn id(self) -> &'static str {
        match self {
            Account::Xai => "xai",
        }
    }

    /// The service's name, for use in a sentence.
    pub fn name(self) -> &'static str {
        match self {
            Account::Xai => "x.ai",
        }
    }

    /// The subscription the account needs, shown beside it in a list of
    /// connections.
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

// From `https://auth.x.ai/.well-known/openid-configuration`.
const XAI_DEVICE_URL: &str = "https://auth.x.ai/oauth2/device/code";
const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";

/// The public client (no secret) of xAI's Grok Build CLI, reused because xAI
/// offers third parties no registration. xAI could withdraw it;
/// `.claude/spec/aldwin-login.md` records the decision.
const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";

/// The inference endpoint checks `grok-cli:access` and `api:access`;
/// without `offline_access` no refresh token is issued.
const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
