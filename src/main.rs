extern crate serde;
extern crate serde_derive;
extern crate serde_json;
extern crate hyper;
extern crate futures;
#[macro_use]
extern crate enum_future;
extern crate tokio_core;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;
#[macro_use]
extern crate configure_me;

mod error;

include_config!();

use std::path::PathBuf;
use std::collections::{HashSet, HashMap};
use futures::Future;
use hyper::{server::Service, Request, Response};
use serde_json::Value as JsonValue;
use hyper::{StatusCode, header::ContentLength, Method, Headers};
use std::rc::Rc;
use slog::Logger;
use self::error::{BadRequest, InvalidType};
use std::borrow::Borrow;
use hyper::header::Authorization;
use hyper::header::Basic as BasicAuth;

type JsonObject = serde_json::Map<String, JsonValue>;
type JsonArray = Vec<JsonValue>;

#[derive(Deserialize)]
pub struct User {
    pub password: String,
    pub allowed_calls: HashSet<String>,
}

pub type Users = HashMap<String, User>;

struct ClientContext<U> {
    users: U,
    auth: Authorization<BasicAuth>,
    logger: Logger,
}

impl<U: Borrow<Users>> ClientContext<U> {
    fn is_method_authorized(&self, method: &str) -> bool {
        if let Some(user) = self.users.borrow().get(&self.auth.username) {
            if self.auth.password.as_ref().map(AsRef::as_ref).unwrap_or("") == user.password && user.allowed_calls.contains(method) {
                debug!(self.logger, "Permitted call"; "method" => method);
                return true;
            }
        }
        error!(self.logger, "Unauthorized call"; "method" => method);
        false
    }

    fn is_obj_authorized(&self, obj: &JsonObject) -> Result<bool, BadRequest> {
        obj.get("method")
            .ok_or(BadRequest::MissingMethod)
            .and_then(|method| match method {
                JsonValue::String(method) => Ok(self.is_method_authorized(&method)),
                JsonValue::Null => Err(BadRequest::InvalidType(InvalidType::Null)),
                JsonValue::Bool(_) => Err(BadRequest::InvalidType(InvalidType::Bool)),
                JsonValue::Number(_) => Err(BadRequest::InvalidType(InvalidType::Number)),
                JsonValue::Array(_) => Err(BadRequest::InvalidType(InvalidType::Array)),
                JsonValue::Object(_) => Err(BadRequest::InvalidType(InvalidType::Object)),
            })
    }

    fn is_arr_authorized(&self, arr: &JsonArray) -> Result<bool, BadRequest> {
        for item in arr {
            let result = match item {
                JsonValue::Null => Err(BadRequest::InvalidType(InvalidType::Null)),
                JsonValue::Bool(_) => Err(BadRequest::InvalidType(InvalidType::Bool)),
                JsonValue::Number(_) => Err(BadRequest::InvalidType(InvalidType::Number)),
                JsonValue::String(_) => Err(BadRequest::InvalidType(InvalidType::String)),
                JsonValue::Array(_) => Err(BadRequest::InvalidType(InvalidType::Array)),
                JsonValue::Object(obj) => self.is_obj_authorized(obj),
            }?;
            if !result {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

type HttpClient = hyper::Client<hyper::client::HttpConnector>;

enum AuthSource {
    Const(Authorization<BasicAuth>),
    CookieFile(PathBuf),
}

impl From<BasicAuth> for AuthSource {
    fn from(value: BasicAuth) -> Self {
        AuthSource::Const(Authorization(value))
    }
}

impl AuthSource {
    fn from_config(user: Option<String>, password: Option<String>, file: Option<PathBuf>) -> Result<Self, &'static str> {
        match (user, password, file) {
            (Some(username), Some(password), None) => Ok(BasicAuth { username, password: Some(password) }.into()),
            (None, Some(password), None) => Ok(BasicAuth { username: Default::default(), password: Some(password) }.into()),
            (None, None, Some(cookie_file)) => Ok(AuthSource::CookieFile(cookie_file)),
            // It could pull it from bitcoin.conf, but I don't think it's worth my time.
            // PRs open.
            (None, None, None) => Err("missing authentication information"),
            _ => Err("either a password and possibly a username or a cookie file must be specified"),
        }
    }

    fn load_from_file(path: &PathBuf) -> Result<Authorization<BasicAuth>, std::io::Error> {
        std::fs::read_to_string(path).map(|cookie| Authorization(BasicAuth { username: cookie, password: None, }))
    }

    fn try_load(&self) -> Result<Authorization<BasicAuth>, std::io::Error> {
        match self {
            AuthSource::Const(auth) => Ok(auth.clone()),
            AuthSource::CookieFile(path) => AuthSource::load_from_file(path),
        }
    }
}

struct Proxy {
    users: HashMap<String, User>,
    auth: AuthSource,
    client: HttpClient,
    dest_uri: hyper::Uri,
}

struct ProxyHandle {
    proxy: Rc<Proxy>,
    logger: Logger,
}

impl ProxyHandle {
    fn new<T: 'static + slog::SendSyncRefUnwindSafeKV>(&self, values: slog::OwnedKV<T>) -> Self {
        ProxyHandle {
            proxy: Rc::clone(&self.proxy),
            logger: self.logger.new(values),
        }
    }
}

impl std::ops::Deref for ProxyHandle {
    type Target = Proxy;

    fn deref(&self) -> &Self::Target {
        &self.proxy
    }
}

impl Service for ProxyHandle {
    type Request = Request;
    type Response = Response;
    type Error = hyper::Error;
    type Future = Box<dyn Future<Item=Self::Response, Error=Self::Error>>;

    fn call(&self, req: Request) -> Self::Future {
        fn send_bad_request() -> impl Future<Item=Response, Error=hyper::Error> {
            const BAD_REQUEST: &str = "{ \"error\" : \"bad request\" }";
            futures::future::ok(Response::new()
                .with_status(StatusCode::BadRequest)
                .with_header(ContentLength(BAD_REQUEST.len() as u64))
                .with_body(BAD_REQUEST))
        }

        fn send_unauthorized() -> impl Future<Item=Response, Error=hyper::Error> {
            const UNAUTHORIZED: &str = "{ \"error\" : \"unauthorized\" }";
            futures::future::ok(Response::new()
                .with_status(StatusCode::Unauthorized)
                .with_header(ContentLength(UNAUTHORIZED.len() as u64))
                .with_body(UNAUTHORIZED))
        }

        fn send_internal_error() -> impl Future<Item=Response, Error=hyper::Error> {
            const INTERNAL_ERROR: &str = "{ \"error\" : \"internal server error\" }";
            futures::future::ok(Response::new()
                .with_status(StatusCode::InternalServerError)
                .with_header(ContentLength(INTERNAL_ERROR.len() as u64))
                .with_body(INTERNAL_ERROR))
        }

        fn forward_call(client: &HttpClient, http_method: Method, uri: hyper::Uri, http_version: hyper::HttpVersion, headers: Headers, body: hyper::Chunk) -> impl Future<Item=Response, Error=hyper::Error> {
            let mut request = Request::new(http_method, uri);
            request.set_version(http_version);
            *request.headers_mut() = headers;
            request.set_body(body);

            client.request(request).map(|mut response| {
                let mut forwarded = Response::new();
                forwarded.set_status(response.status());
                std::mem::swap(forwarded.headers_mut(), response.headers_mut());
                forwarded.set_body(response.body());

                forwarded
            })
        }

        let (http_method, uri, http_version, mut headers, body) = req.deconstruct();

        if http_method == Method::Post && uri.path() == "/" {
            if let Some(auth) = headers.remove::<hyper::header::Authorization<hyper::header::Basic>>() {
                use futures::Stream;

                let this = self.new(o!("user" => auth.username.clone()));
                Box::new(body.concat2().and_then(move |body| {
                    enum_future!(Ret, Forward, Unauthorized, BadRequest, InternalError);

                    let ctx = ClientContext {
                        users: &this.users,
                        auth,
                        logger: this.logger.clone(),
                    };

                    let request = serde_json::from_slice::<serde_json::Value>(body.as_ref());

                    let result = match request {
                        Ok(JsonValue::Object(obj)) => ctx.is_obj_authorized(&obj),
                        Ok(JsonValue::Array(arr)) => ctx.is_arr_authorized(&arr),
                        Ok(JsonValue::String(_)) => Err(BadRequest::InvalidType(InvalidType::String)),
                        Ok(JsonValue::Null) => Err(BadRequest::InvalidType(InvalidType::Null)),
                        Ok(JsonValue::Bool(_)) => Err(BadRequest::InvalidType(InvalidType::Bool)),
                        Ok(JsonValue::Number(_)) => Err(BadRequest::InvalidType(InvalidType::Number)),
                        Err(error) => Err(BadRequest::Json(error)),
                    };

                    match result {
                        Ok(true) => {
                            let logger = &this.logger;
                            this
                                .auth
                                .try_load()
                                .map(|auth| {
                                    headers.set(auth);
                                    Ret::Forward(forward_call(&this.client, http_method, this.dest_uri.clone(), http_version, headers, body))
                                })
                                .map_err(|err| {
                                    if err.kind() != std::io::ErrorKind::NotFound {
                                        error!(logger, "Failed to read cookie file: {}", err);
                                    }

                                    send_internal_error()
                                })
                                .unwrap_or_else(Ret::InternalError)
                        },
                        Ok(false) => {
                            Ret::Unauthorized(send_unauthorized())
                        },
                        Err(error) => {
                            error!(this.logger, "Bad request"; "error" => %error);
                            Ret::BadRequest(send_bad_request())
                        },
                    }
                }))
            } else {
                error!(self.logger, "User unauthorized");
                Box::new(send_unauthorized())
            }
        } else {
            error!(self.logger, "Bad request");
            Box::new(send_bad_request())
        }
    }
}

fn main() {
    use futures::Stream;
    use slog::Drain;

    let (config, _) = config::Config::including_optional_config_files(std::iter::empty::<&str>()).unwrap_or_exit();
    let auth = AuthSource::from_config(config.bitcoind_user, config.bitcoind_password, config.cookie_file)
        .unwrap_or_else(|msg| {
            eprintln!("Configuration error: {}", msg);
            std::process::exit(1);
        });

    let dest_uri = format!("http://{}:{}", config.bitcoind_address, config.bitcoind_port).parse().unwrap();

    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let logger = slog::Logger::root(drain, o!());

    let mut core = tokio_core::reactor::Core::new().unwrap();
    let handle = core.handle();

    let addr = std::net::SocketAddr::new(config.bind_address, config.bind_port);
    info!(logger, "Binding"; "bind address" => addr);
    let listener = tokio_core::net::TcpListener::bind(&addr, &handle).unwrap();
    let proxy = Proxy {
        auth,
        users: config.user,
        client: HttpClient::new(&handle),
        dest_uri,
    };
    let service = ProxyHandle { proxy: Rc::new(proxy), logger: logger.new(o!()) };

    let incoming = listener
        .incoming()
        .map_err(|err| error!(logger, "Failed to accept connection"; "error" => %err))
        .map(move |(socket, addr)| {
            let service = service.new(o!("client address" => addr));
            info!(service.logger, "Connected client");
            let err_logger = service.logger.new(o!());
            hyper::server::Http::<hyper::Chunk>::new()
                .serve_connection(socket, service)
                .map_err(move |err| error!(err_logger, "Connection encountered an error"; "error" => %err))
        });

    let server = incoming.for_each(move |connection| {
        handle.spawn(connection);
        Ok(())
    });
    core.run(server).unwrap();
}
