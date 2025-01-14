//! Authentication towards the API.
use std::sync::{Arc, Mutex, MutexGuard};

use crate::reddit::{Error, Result};

use reqwest::{
    blocking::{Client, Response},
    header, StatusCode,
};
use serde::{Deserialize, Serialize};

/// Behavior of something that can provide access to the Reddit API.
pub trait Authenticator {
    /// Refresh/fetch the token from the Reddit API.
    fn login(&mut self) -> Result<()>;
    /// Provide a token to authenticate to the reddit API with.
    /// If this is invalid(outdated) or None, [`login`] should refresh it.
    fn token(&self) -> Option<Token>;
    /// This authenticator can make requests that pertain to a user, such as posting a comment etc.
    fn is_user(&self) -> bool;

    fn default_agent() -> String {
        format!(
            "{}:{}:{}:{}",
            "desktop",
            "snew",
            env!("CARGO_PKG_VERSION"),
            "(by snewAuthenticator)"
        )
    }
}

/// An access token.
#[derive(Debug, Clone, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub expires_in: i32,
    scope: String,
    token_type: String,
}

/// Authenticated interaction with the Reddit API. Use [`crate::reddit::Reddit`] instead.
/// This is shared by all current interactors with what reddit calls 'things', so they can make requests for more posts, comments, etc.
#[derive(Debug, Clone)]
pub struct AuthenticatedClient<T: Authenticator> {
    pub(crate) client: Arc<Mutex<Client>>,
    pub(crate) authenticator: Arc<Mutex<T>>,
    user_agent: String,
}

impl<T: Authenticator> AuthenticatedClient<T> {
    pub fn new(mut authenticator: T, user_agent: &str) -> Result<Self> {
        authenticator.login()?;

        if let Some(token) = authenticator.token() {
            let client = Self::make_client(user_agent, &token.access_token)?;
            Ok(Self {
                authenticator: Arc::new(Mutex::new(authenticator)),
                client: Arc::new(Mutex::new(client)),
                user_agent: String::from(user_agent),
            })
        } else {
            // Pretty sure this can never happen, but better safe than sorry? :D
            Err(Error::AuthenticationError(String::from("Token was not set after logging in, but no error was returned. Report bug at https://github.com/Zower/snew")))
        }
    }

    /// Make a get request to `url`
    /// Errors if the status code was unexpected, the client cannot re-initialize or make the request, or if the authentication fails.
    pub fn get<Q: Serialize>(&self, url: &str, queries: Option<&Q>) -> Result<Response> {
        // Make one request
        let mut client = self
            .client
            .lock()
            .expect("Poisoned mutex, report bug at https://github.com/Zower/snew");

        let response = self.make_request(&client, url, queries)?;

        // Check if the request was successful
        if self.check_auth(&response)? {
            Ok(response)
        } else {
            // Refresh token
            let mut authenticator = self
                .authenticator
                .lock()
                .expect("Poisoned mutex, report bug at https://github.com/Zower/snew");
            authenticator.login()?;

            if let Some(token) = authenticator.token() {
                // Create a new client with correct token
                *client = Self::make_client(&self.user_agent, &token.access_token)?;
            } else {
                // Pretty sure this can never happen, but better safe than sorry? :D
                return Err(Error::AuthenticationError(String::from("Token was not set after logging in, but no error was returned. Report bug at https://github.com/Zower/snew")));
            }

            let response = self.make_request(&client, url, queries)?;

            if response.status() == StatusCode::OK {
                Ok(response)
            }
            // Still not authenticated correctly
            else {
                Err(Error::AuthenticationError(String::from(
                    "Failed to authenticate, even after requesting new token. Check credentials.",
                )))
            }
        }
    }

    // Checks queries and makes the actual web request
    fn make_request<Q: Serialize>(
        &self,
        client: &MutexGuard<Client>,
        url: &str,
        queries: Option<&Q>,
    ) -> Result<Response> {
        if let Some(queries) = queries {
            Ok(client.get(url).query(queries).send()?)
        } else {
            Ok(client.get(url).send()?)
        }
    }

    // Checks that the response is OK. Errors if status code is not expected.
    fn check_auth(&self, response: &Response) -> Result<bool> {
        let status = response.status();

        if status == StatusCode::OK {
            Ok(true)
        } else if status == StatusCode::FORBIDDEN || status == StatusCode::UNAUTHORIZED {
            Ok(false)
        } else {
            return Err(Error::AuthenticationError(format!(
                "Reddit returned an unexpected code: {}",
                status
            )));
        }
    }

    // Make a reqwest client with user_agent and bearer token set as default headers.
    fn make_client(user_agent: &str, access_token: &str) -> Result<Client> {
        let mut headers = header::HeaderMap::new();

        let mut authorization =
            header::HeaderValue::from_str(&format!("bearer {}", &access_token))?;

        authorization.set_sensitive(true);

        headers.insert(header::AUTHORIZATION, authorization);

        Ok(Client::builder()
            .user_agent(user_agent)
            .default_headers(headers)
            .build()?)
    }
}

/// Client ID and Secret for the application
#[derive(Debug, Clone)]
pub struct ClientInfo {
    pub client_id: String,
    pub client_secret: String,
}

/// Login credentials
#[derive(Debug, Clone)]
pub struct Credentials {
    client_info: ClientInfo,
    pub username: String,
    pub password: String,
}

impl Credentials {
    pub fn new(client_id: &str, client_secret: &str, username: &str, password: &str) -> Self {
        Self {
            client_info: ClientInfo {
                client_id: String::from(client_id),
                client_secret: String::from(client_secret),
            },
            username: String::from(username),
            password: String::from(password),
        }
    }
}

/// Authenticator for Script applications.
/// This includes username and password, which means you are logged in, and can perform actions such as voting.
///See also reddit OAuth API docs.
#[derive(Debug, Clone)]
pub struct ScriptAuthenticator {
    creds: Credentials,
    token: Option<Token>,
}

impl ScriptAuthenticator {
    pub fn new(creds: Credentials) -> Self {
        Self { creds, token: None }
    }
}

impl Authenticator for ScriptAuthenticator {
    fn login(&mut self) -> Result<()> {
        let client = Client::builder()
            .user_agent(ScriptAuthenticator::default_agent())
            .build()?;

        // Make the request for the access token.
        let response = client
            .post("https://www.reddit.com/api/v1/access_token")
            .query(&[
                ("grant_type", "password"),
                ("username", &self.creds.username),
                ("password", &self.creds.password),
            ])
            .basic_auth(
                self.creds.client_info.client_id.clone(),
                Some(self.creds.client_info.client_secret.clone()),
            )
            .send()?;

        let status = response.status();
        let text = response.text()?;
        let slice = &text;

        // Parse the response as JSON.
        if let Ok(token) = serde_json::from_str::<Token>(slice) {
            self.token = Some(token);
        }
        // Various errors that can occur
        else if let Ok(error) = serde_json::from_str::<OkButError>(slice) {
            return Err(Error::AuthenticationError(format!(
                "{}, Reddit returned: {}",
                "Username or password are most likely wrong", error.error
            )));
        } else if status == StatusCode::UNAUTHORIZED {
            return Err(Error::AuthenticationError(String::from(
                "Client ID or Secret are wrong. Reddit returned 401 Unauthorized",
            )));
        }
        // Unknown what went wrong
        else {
            return Err(Error::AuthenticationError(format!(
                "Unexpected error occured, text: {}, code: {}",
                text, &status
            )));
        }
        Ok(())
    }

    fn token(&self) -> Option<Token> {
        self.token.clone()
    }

    fn is_user(&self) -> bool {
        true
    }
}

/// Anonymous authentication.
/// You will still need a client ID and secret, but you will not be logged in as some user. You can browse reddit, but not e.g. vote.
#[derive(Debug, Clone)]
pub struct ApplicationAuthenticator {
    client_info: ClientInfo,
    token: Option<Token>,
}

impl ApplicationAuthenticator {
    pub fn new(client_id: &str, client_secret: &str) -> Self {
        Self {
            client_info: ClientInfo {
                client_id: String::from(client_id),
                client_secret: String::from(client_secret),
            },
            token: None,
        }
    }
}

impl Authenticator for ApplicationAuthenticator {
    fn login(&mut self) -> Result<()> {
        let client = Client::builder()
            .user_agent(ApplicationAuthenticator::default_agent())
            .build()?;

        // Make the request for the access token.
        let response = client
            .post("https://www.reddit.com/api/v1/access_token")
            .query(&[("grant_type", "client_credentials")])
            .basic_auth(
                self.client_info.client_id.clone(),
                Some(self.client_info.client_secret.clone()),
            )
            .send()?;

        let status = response.status();
        let text = response.text()?;
        let slice = &text;

        // Parse the response as JSON.
        if let Ok(token) = serde_json::from_str::<Token>(slice) {
            self.token = Some(token);
        }
        // Various errors that can occur
        else if let Ok(error) = serde_json::from_str::<OkButError>(slice) {
            return Err(Error::AuthenticationError(format!(
                "{}, Reddit returned: {}",
                "Username or password are most likely wrong", error.error
            )));
        } else if status == StatusCode::UNAUTHORIZED {
            return Err(Error::AuthenticationError(String::from(
                "Client ID or Secret are wrong. Reddit returned 401 Unauthorized",
            )));
        }
        // Unknown what went wrong
        else {
            return Err(Error::AuthenticationError(format!(
                "Unexpected error occured, text: {}, code: {}",
                text, &status
            )));
        }
        Ok(())
    }
    fn token(&self) -> Option<Token> {
        self.token.clone()
    }

    fn is_user(&self) -> bool {
        false
    }
}

// Reddit can return 200 OK even if the credentials are wrong, in which case it will include one field, "error": "message"
#[derive(Deserialize)]
struct OkButError {
    error: String,
}
