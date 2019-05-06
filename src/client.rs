// Copyright (C) 2019 Daniel Mueller <deso@posteo.net>
// SPDX-License-Identifier: GPL-3.0-or-later

use std::str::from_utf8;

use futures::future::Future;
use futures::stream::Stream;

use hyper::Body;
use hyper::Client as HttpClient;
use hyper::client::HttpConnector;
use hyper::http::StatusCode;
use hyper_tls::HttpsConnector;

use log::debug;
use log::info;
use log::Level::Debug;
use log::log_enabled;

use url::Url;

use crate::endpoint::ConvertResult;
use crate::endpoint::Endpoint;
use crate::env::api_info;
use crate::Error;
use crate::events::EventStream;
use crate::events::stream;


/// A `Client` is the entity used by clients of this module for
/// interacting with the Alpaca API. It provides the highest-level
/// primitives and also implements the `Trader` trait, which abstracts
/// away the trading related functionality common among all supported
/// services.
#[derive(Debug)]
pub struct Client {
  api_base: Url,
  key_id: Vec<u8>,
  secret: Vec<u8>,
  client: HttpClient<HttpsConnector<HttpConnector>, Body>,
}

impl Client {
  /// Create a new `Client` using the given key ID and secret for
  /// connecting to the API.
  fn new(api_base: Url, key_id: Vec<u8>, secret: Vec<u8>) -> Result<Self, Error> {
    // So here is the deal. In tests we use the block_on_all function to
    // wait for futures. This function waits until *all* spawned futures
    // completed. Now, by virtue of keeping idle connections around --
    // which effectively map to spawned tasks -- we will block until
    // those connections die. We can't have that happen for tests, so we
    // disable idle connections for them.
    // While at it, also use the minimum number of threads for the
    // `HttpsConnector`.
    #[cfg(test)]
    fn client() -> Result<HttpClient<HttpsConnector<HttpConnector>, Body>, Error> {
      let https = HttpsConnector::new(1)?;
      let client = HttpClient::builder()
        .max_idle_per_host(0)
        .build::<_, Body>(https);
      Ok(client)
    }
    #[cfg(not(test))]
    fn client() -> Result<HttpClient<HttpsConnector<HttpConnector>, Body>, Error> {
      let https = HttpsConnector::new(4)?;
      let client = HttpClient::builder().build::<_, Body>(https);
      Ok(client)
    }

    Ok(Self {
      api_base,
      key_id,
      secret,
      client: client()?,
    })
  }

  /// Create a new `Client` with information from the environment.
  pub fn from_env() -> Result<Self, Error> {
    let (api_base, key_id, secret) = api_info()?;
    Self::new(api_base, key_id, secret)
  }

  /// Create and issue a request and decode the response.
  pub fn issue<R>(
    &self,
    input: R::Input,
  ) -> Result<impl Future<Item = R::Output, Error = R::Error>, Error>
  where
    R: Endpoint,
    R::Output: Send + 'static,
    R::Error: From<hyper::Error> + Send + 'static,
    ConvertResult<R::Output, R::Error>: From<(StatusCode, Vec<u8>)>,
  {
    let req = R::request(&self.api_base, &self.key_id, &self.secret, &input)?;
    if log_enabled!(Debug) {
      debug!("HTTP request: {:?}", req);
    } else {
      info!("HTTP request: {} to {}", req.method(), req.uri());
    }

    let fut = self
      .client
      .request(req)
      .and_then(|res| {
        let status = res.status();
        // We unconditionally wait for the full body to be received
        // before even evaluating the header. That is mostly done for
        // simplicity and it shouldn't really matter anyway because most
        // if not all requests evaluate the body on success and on error
        // the server shouldn't send back much.
        // TODO: However, there may be one case that has the potential
        //       to cause trouble: when we receive, for example, the
        //       list of all orders it now needs to be stored in memory
        //       in its entirety. That may blow things.
        res.into_body().concat2().map(move |body| (status, body))
      })
      .map_err(R::Error::from)
      .and_then(|(status, body)| {
        let bytes = body.into_bytes();
        let body = Vec::from(bytes.as_ref());

        info!("HTTP status: {}", status);
        if log_enabled!(Debug) {
          match from_utf8(&body) {
            Ok(s) => debug!("HTTP body: {}", s),
            Err(b) => debug!("HTTP body: {}", b),
          }
        }

        let res = ConvertResult::<R::Output, R::Error>::from((status, body));
        Into::<Result<_, _>>::into(res)
      });

    Ok(Box::new(fut))
  }

  /// Subscribe to the given stream in order to receive updates.
  pub fn subscribe<S>(
    &self,
  ) -> impl Future<Item = impl Stream<Item = S::Event, Error = Error>, Error = Error>
  where
    S: EventStream,
  {
    stream::<S>(
      self.api_base.clone(),
      self.key_id.clone(),
      self.secret.clone(),
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  use tokio::runtime::current_thread::block_on_all;

  use test_env_log::test;

  use crate::Str;


  #[derive(Debug)]
  pub struct GetNotFound {}

  EndpointDef! {
    GetNotFound,
    Ok => (), GetNotFoundOk, [],
    Err => GetNotFoundError, []
  }

  impl Endpoint for GetNotFound {
    type Input = ();
    type Output = GetNotFoundOk;
    type Error = GetNotFoundError;

    fn path(_input: &Self::Input) -> Str {
      "/v1/foobarbaz".into()
    }
  }


  #[test]
  fn unexpected_status_code_return() -> Result<(), Error> {
    let client = Client::from_env()?;
    let future = client.issue::<GetNotFound>(())?;
    let err = block_on_all(future).unwrap_err();

    match err {
      GetNotFoundError::UnexpectedStatus(status) => {
        assert_eq!(status, StatusCode::NOT_FOUND);
      },
      _ => panic!("Received unexpected error: {:?}", err),
    };
    Ok(())
  }
}