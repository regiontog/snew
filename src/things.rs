//! Reddit 'things'. In the API, a thing is a type + fullname.
use serde::Deserialize;

use self::raw::{generic_kind::RawKind, listing::RawListing, post::RawPostData};
use crate::{
    auth::{AuthenticatedClient, Authenticator},
    reddit::{Error, Result},
};

/// A handle to interact with a subreddit.
/// See [`PostFeed`] for some gotchas when iterating over Posts.
#[derive(Debug)]
pub struct Subreddit<'a, T: Authenticator> {
    pub url: String,
    client: &'a AuthenticatedClient<T>,
}

impl<'a, T: Authenticator> Subreddit<'a, T> {
    pub fn create(url: &str, client: &'a AuthenticatedClient<T>) -> Self {
        Self {
            url: String::from(url),
            client,
        }
    }
    pub fn hot(&self) -> PostFeed<T> {
        self.posts_sorted("hot")
    }
    #[allow(clippy::clippy::new_ret_no_self)]
    pub fn new(&self) -> PostFeed<T> {
        self.posts_sorted("new")
    }
    pub fn random(&self) -> PostFeed<T> {
        self.posts_sorted("random")
    }
    pub fn rising(&self) -> PostFeed<T> {
        self.posts_sorted("rising")
    }
    pub fn top(&self) -> PostFeed<T> {
        self.posts_sorted("top")
    }

    fn posts_sorted(&self, path: &str) -> PostFeed<T> {
        PostFeed {
            limit: 100,
            url: format!("{}/{}", self.url, path),
            cached_posts: Vec::new(),
            client: self.client,
            after: String::from(""),
        }
    }
}

/// A post.
#[derive(Debug, Clone)]
pub struct Post<'a, T: Authenticator> {
    client: &'a AuthenticatedClient<T>,
    pub title: String,
    /// Upvotes.
    pub ups: i32,
    /// Downvotes.
    pub downs: i32,
    /// The associated URL of this post. It is an external website if the post is a link, otherwise the comment section.
    pub url: String,
    /// The author.
    pub author: String,
    /// The text of this post.
    pub selftext: String,
    /// The unique base 36 ID of this post
    pub id: String,
    /// The 'kind'. This should always be t3. Combine with [`Self::id`] to get the fullname of this post.
    pub kind: String,
}

impl<'a, T: Authenticator> Post<'a, T> {
    pub fn comments(&self) -> CommentFeed<T> {
        CommentFeed {
            client: self.client,
            url: self.url.clone(),
            // url: format!("{}/comments/{}", self.url),
            cached_comments: Vec::new(),
        }
    }
}

/// Represents interacting with a set of posts, meant to be iterated over. As long as there are posts to iterate over, this iterator will continue. You may wish to take() some elements.
/// The iterator returns a Result<Post, Error>. The errors are either from the HTTP request or the JSON parsing.
#[derive(Debug)]
pub struct PostFeed<'a, T: Authenticator> {
    /// The amount of posts to request from the Reddit API. This does not mean you can only iterate over this many posts.
    /// The Iterator will simply make more requests if you iterate over more than this limit.
    /// You should set this to a specific number if you know that you will be making some exact number of requests < 100, so
    /// the iterator doesnt fetch more posts than it needs to. If you dont know how many you are iterating over, just leave it at the default
    /// which is 100, the max Reddit allows.
    pub limit: i32,
    url: String,
    cached_posts: Vec<Post<'a, T>>,
    client: &'a AuthenticatedClient<T>,
    after: String,
}

impl<'a, T: Authenticator> Iterator for PostFeed<'a, T> {
    type Item = Result<Post<'a, T>>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(post) = self.cached_posts.pop() {
            Some(Ok(post))
        } else {
            let res = self.client.get(
                self.url.as_str(),
                Some(&[
                    ("limit", self.limit.to_string()),
                    ("after", self.after.clone()),
                ]),
            );
            // Probably some cleaner way to do this
            let listing = match res {
                Ok(response) => match response.text() {
                    Ok(text) => match serde_json::from_str::<RawListing<RawKind<RawPostData>>>(
                        text.as_str(),
                    ) {
                        Ok(raw) => raw,
                        Err(err) => return Some(Err(Error::APIParseError(err))),
                    },
                    Err(err) => return Some(Err(Error::RequestError(err))),
                },
                Err(err) => return Some(Err(err)),
            };

            // Make sure the next HTTP request gets posts after the last one we fetched.
            self.after = listing.data.pagination.after;

            let client = &self.client;

            // Add posts to the cached_posts array, converting from RawPost to Post in the process
            self.cached_posts.extend(
                listing
                    .data
                    .children
                    .into_iter()
                    .rev()
                    .map(|raw| (raw, *client))
                    .map(From::from),
            );

            let post = self.cached_posts.pop();
            post.map(Ok)
        }
    }
}

/// A comment.
#[derive(Debug)]
pub struct Comment {
    pub author: String,
}

#[derive(Debug)]
pub struct CommentFeed<'a, T: Authenticator> {
    url: String,
    client: &'a AuthenticatedClient<T>,
    cached_comments: Vec<Comment>,
}

/// Information about the authenticated user
#[derive(Debug, Deserialize)]
pub struct Me {
    pub name: String,
    pub total_karma: i32,
    pub link_karma: i32,
    pub comment_karma: i32,
    pub verified: bool,
}

// Create a post from som raw data.
impl<'a, T: Authenticator> From<(RawKind<RawPostData>, &'a AuthenticatedClient<T>)>
    for Post<'a, T>
{
    fn from(raw: (RawKind<RawPostData>, &'a AuthenticatedClient<T>)) -> Self {
        let (raw, client) = raw;
        Self {
            client,
            title: raw.data.title,
            ups: raw.data.ups,
            downs: raw.data.downs,
            url: raw.data.url,
            author: raw.data.author,
            selftext: raw.data.selftext,
            id: raw.data.id,
            kind: raw.kind,
        }
    }
}

// Not used yet
// pub enum Kind {
//     Comment,
//     Account,
//     Link,
//     Message,
//     Subreddit,
//     Award,
// }

// impl std::convert::TryFrom<&str> for Kind {
//     type Error = crate::reddit::Error;

//     fn try_from(value: &str) -> Result<Self, Self::Error> {
//         match value {
//             "t1" => Ok(Self::Comment),
//             _ => Err(crate::reddit::Error::KindParseError),
//         }
//     }
// }

// The raw responses from Reddit. The interpreted structs like [`crate::things::Subreddit`] and [`crate::things::Post`] are meant to be used instead of these, and should cover regular usecases.
#[doc(hidden)]
pub mod raw {
    use serde::Deserialize;

    #[derive(Debug, Clone, Deserialize)]
    pub struct Pagination {
        pub after: String,
        // pub before: String,
    }

    pub mod listing {
        use super::Pagination;
        use serde::Deserialize;

        // Listings from Reddit take this form.
        #[derive(Debug, Clone, Deserialize)]
        pub struct RawListing<T> {
            pub data: RawListingData<T>,
        }

        #[derive(Debug, Clone, Deserialize)]
        pub struct RawListingData<T> {
            #[serde(flatten)]
            pub pagination: Pagination,
            pub children: Vec<T>,
        }
    }

    pub mod generic_kind {
        use serde::Deserialize;

        #[derive(Debug, Deserialize)]
        pub struct RawKind<T> {
            pub data: T,
            pub kind: String,
        }
    }

    pub mod post {
        use serde::Deserialize;

        #[derive(Debug, Clone, Deserialize)]
        pub struct RawPostData {
            pub title: String,
            pub ups: i32,
            pub downs: i32,
            pub url: String,
            pub author: String,
            pub selftext: String,
            pub id: String,
        }
    }

    pub mod comment {
        use serde::Deserialize;

        #[derive(Debug, Deserialize)]
        pub struct RawComment {}
    }
}
