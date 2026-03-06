//! HTTP service wrapper for hyper.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;

use bytes::Bytes;
use http::{Request, Response};

use crate::admin_router::AdminRouter;
use crate::ingress_router::IngressRouter;
use crate::router::Router;

pub type BoxBody = http_body_util::combinators::BoxBody<Bytes, Infallible>;

/// Trait for routers that can handle HTTP requests.
pub trait HttpRouter: Clone + Send + Sync + 'static {
    fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> impl Future<Output = Result<Response<BoxBody>, Infallible>> + Send;
}

// Implement HttpRouter for all router types
impl HttpRouter for Router {
    fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> impl Future<Output = Result<Response<BoxBody>, Infallible>> + Send {
        Router::handle(self, req)
    }
}

impl HttpRouter for AdminRouter {
    fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> impl Future<Output = Result<Response<BoxBody>, Infallible>> + Send {
        AdminRouter::handle(self, req)
    }
}

impl HttpRouter for IngressRouter {
    fn handle(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> impl Future<Output = Result<Response<BoxBody>, Infallible>> + Send {
        IngressRouter::handle(self, req)
    }
}

/// Generic Tower Service wrapper around any router.
///
/// This implements `hyper::service::Service` for use with `hyper_util`'s connection builder.
#[derive(Clone)]
pub struct EdgeService<R: HttpRouter> {
    router: R,
}

impl<R: HttpRouter> EdgeService<R> {
    pub fn new(router: R) -> Self {
        Self { router }
    }
}

impl<R: HttpRouter> hyper::service::Service<Request<hyper::body::Incoming>> for EdgeService<R> {
    type Response = Response<BoxBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, req: Request<hyper::body::Incoming>) -> Self::Future {
        let router = self.router.clone();
        Box::pin(async move { router.handle(req).await })
    }
}
