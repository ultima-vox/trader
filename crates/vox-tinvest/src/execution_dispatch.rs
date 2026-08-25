//! Durable capital-mutation boundary. UNKNOWN is committed before transport send.

use std::future::Future;

use thiserror::Error;
use tonic::Code;
use vox_domain::{
    AuthoritativeMutationOutcome, BrokerOrderId, BrokerStopOrderId, ClientRequestId, Environment,
    IdentityError, MutationAuthorization, MutationDecision, MutationEvidence,
    MutationEvidenceStore, MutationRecovery, StoreError,
};

use crate::generated::v1;
use crate::{GrpcError, GrpcErrorKind, GrpcResponse, TInvestGrpcClient};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionRoute {
    Production,
    Sandbox,
}

impl ExecutionRoute {
    const fn environment(self) -> Environment {
        match self {
            Self::Production => Environment::Live,
            Self::Sandbox => Environment::Sandbox,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AcknowledgedMutation<T> {
    pub response: GrpcResponse<T>,
    pub evidence: MutationEvidence,
}

pub struct ExecutionMutationDispatcher<S> {
    recovery: MutationRecovery<S>,
}

impl<S: MutationEvidenceStore> ExecutionMutationDispatcher<S> {
    #[must_use]
    pub const fn new(store: S) -> Self {
        Self {
            recovery: MutationRecovery::new(store),
        }
    }

    pub fn decision(&self, id: &ClientRequestId) -> Result<MutationDecision, StoreError> {
        self.recovery.decision(id)
    }

    #[must_use]
    pub fn into_store(self) -> S {
        self.recovery.into_store()
    }

    pub async fn post_order(
        &mut self,
        client: &TInvestGrpcClient,
        authorization: MutationAuthorization,
        route: ExecutionRoute,
        request: v1::PostOrderRequest,
    ) -> Result<AcknowledgedMutation<v1::PostOrderResponse>, ExecutionDispatchError> {
        let logical_id = request.order_id.clone();
        self.execute(
            client,
            authorization.environment(),
            route,
            logical_id,
            || async move {
                match route {
                    ExecutionRoute::Production => client.post_order(authorization, request).await,
                    ExecutionRoute::Sandbox => {
                        client.post_sandbox_order(authorization, request).await
                    }
                }
            },
            |evidence, response| optional_broker_order(evidence, &response.order_id),
        )
        .await
    }

    pub async fn post_order_async(
        &mut self,
        client: &TInvestGrpcClient,
        authorization: MutationAuthorization,
        route: ExecutionRoute,
        request: v1::PostOrderAsyncRequest,
    ) -> Result<AcknowledgedMutation<v1::PostOrderAsyncResponse>, ExecutionDispatchError> {
        let logical_id = request.order_id.clone();
        self.execute(
            client,
            authorization.environment(),
            route,
            logical_id,
            || async move {
                match route {
                    ExecutionRoute::Production => {
                        client.post_order_async(authorization, request).await
                    }
                    ExecutionRoute::Sandbox => {
                        client
                            .post_sandbox_order_async(authorization, request)
                            .await
                    }
                }
            },
            |evidence, response| {
                Ok(
                    match response
                        .trade_intent_id
                        .as_ref()
                        .filter(|value| !value.trim().is_empty())
                    {
                        Some(value) => evidence.with_provider_operation_id(value.clone()),
                        None => evidence,
                    },
                )
            },
        )
        .await
    }

    pub async fn replace_order(
        &mut self,
        client: &TInvestGrpcClient,
        authorization: MutationAuthorization,
        route: ExecutionRoute,
        request: v1::ReplaceOrderRequest,
    ) -> Result<AcknowledgedMutation<v1::PostOrderResponse>, ExecutionDispatchError> {
        let logical_id = request.idempotency_key.clone();
        self.execute(
            client,
            authorization.environment(),
            route,
            logical_id,
            || async move {
                match route {
                    ExecutionRoute::Production => {
                        client.replace_order(authorization, request).await
                    }
                    ExecutionRoute::Sandbox => {
                        client.replace_sandbox_order(authorization, request).await
                    }
                }
            },
            |evidence, response| optional_broker_order(evidence, &response.order_id),
        )
        .await
    }

    pub async fn cancel_order(
        &mut self,
        client: &TInvestGrpcClient,
        authorization: MutationAuthorization,
        route: ExecutionRoute,
        logical_request_id: ClientRequestId,
        request: v1::CancelOrderRequest,
    ) -> Result<AcknowledgedMutation<v1::CancelOrderResponse>, ExecutionDispatchError> {
        self.execute_with_id(
            client,
            authorization.environment(),
            route,
            logical_request_id,
            || async move {
                match route {
                    ExecutionRoute::Production => client.cancel_order(authorization, request).await,
                    ExecutionRoute::Sandbox => {
                        client.cancel_sandbox_order(authorization, request).await
                    }
                }
            },
            |evidence, _| Ok(evidence),
        )
        .await
    }

    pub async fn post_stop_order(
        &mut self,
        client: &TInvestGrpcClient,
        authorization: MutationAuthorization,
        route: ExecutionRoute,
        request: v1::PostStopOrderRequest,
    ) -> Result<AcknowledgedMutation<v1::PostStopOrderResponse>, ExecutionDispatchError> {
        let logical_id = request.order_id.clone();
        self.execute(
            client,
            authorization.environment(),
            route,
            logical_id,
            || async move {
                match route {
                    ExecutionRoute::Production => {
                        client.post_stop_order(authorization, request).await
                    }
                    ExecutionRoute::Sandbox => {
                        client.post_sandbox_stop_order(authorization, request).await
                    }
                }
            },
            |evidence, response| optional_broker_stop_order(evidence, &response.stop_order_id),
        )
        .await
    }

    pub async fn cancel_stop_order(
        &mut self,
        client: &TInvestGrpcClient,
        authorization: MutationAuthorization,
        route: ExecutionRoute,
        logical_request_id: ClientRequestId,
        request: v1::CancelStopOrderRequest,
    ) -> Result<AcknowledgedMutation<v1::CancelStopOrderResponse>, ExecutionDispatchError> {
        self.execute_with_id(
            client,
            authorization.environment(),
            route,
            logical_request_id,
            || async move {
                match route {
                    ExecutionRoute::Production => {
                        client.cancel_stop_order(authorization, request).await
                    }
                    ExecutionRoute::Sandbox => {
                        client
                            .cancel_sandbox_stop_order(authorization, request)
                            .await
                    }
                }
            },
            |evidence, _| Ok(evidence),
        )
        .await
    }

    async fn execute<T, Dispatch, Fut, Attach>(
        &mut self,
        client: &TInvestGrpcClient,
        authorization_environment: Environment,
        route: ExecutionRoute,
        logical_id: String,
        dispatch: Dispatch,
        attach: Attach,
    ) -> Result<AcknowledgedMutation<T>, ExecutionDispatchError>
    where
        Dispatch: FnOnce() -> Fut,
        Fut: Future<Output = Result<GrpcResponse<T>, GrpcError>>,
        Attach: FnOnce(MutationEvidence, &T) -> Result<MutationEvidence, IdentityError>,
    {
        let logical_id = ClientRequestId::new(logical_id)?;
        self.execute_with_id(
            client,
            authorization_environment,
            route,
            logical_id,
            dispatch,
            attach,
        )
        .await
    }

    async fn execute_with_id<T, Dispatch, Fut, Attach>(
        &mut self,
        client: &TInvestGrpcClient,
        authorization_environment: Environment,
        route: ExecutionRoute,
        logical_id: ClientRequestId,
        dispatch: Dispatch,
        attach: Attach,
    ) -> Result<AcknowledgedMutation<T>, ExecutionDispatchError>
    where
        Dispatch: FnOnce() -> Fut,
        Fut: Future<Output = Result<GrpcResponse<T>, GrpcError>>,
        Attach: FnOnce(MutationEvidence, &T) -> Result<MutationEvidence, IdentityError>,
    {
        if route.environment() != client.environment()
            || authorization_environment != client.environment()
        {
            return Err(ExecutionDispatchError::EnvironmentMismatch);
        }
        if self.recovery.decision(&logical_id)? != MutationDecision::Submit {
            return Err(ExecutionDispatchError::ReconciliationRequired);
        }
        let evidence = self.recovery.persist_before_dispatch(logical_id, None)?;
        match dispatch().await {
            Ok(response) => {
                let evidence = attach(evidence, &response.body)?;
                let evidence = self.recovery.persist_authoritative_outcome(
                    evidence,
                    AuthoritativeMutationOutcome::Accepted,
                )?;
                Ok(AcknowledgedMutation { response, evidence })
            }
            Err(source) if authoritative_rejection(&source) => {
                let evidence = self.recovery.persist_authoritative_outcome(
                    evidence,
                    AuthoritativeMutationOutcome::Rejected,
                )?;
                Err(ExecutionDispatchError::Rejected { source, evidence })
            }
            Err(source) => Err(ExecutionDispatchError::UnknownAfterDispatch { source, evidence }),
        }
    }
}

fn optional_broker_order(
    evidence: MutationEvidence,
    value: &str,
) -> Result<MutationEvidence, IdentityError> {
    if value.trim().is_empty() {
        Ok(evidence)
    } else {
        BrokerOrderId::new(value).map(|id| evidence.with_broker_order_id(id))
    }
}

fn optional_broker_stop_order(
    evidence: MutationEvidence,
    value: &str,
) -> Result<MutationEvidence, IdentityError> {
    if value.trim().is_empty() {
        Ok(evidence)
    } else {
        BrokerStopOrderId::new(value).map(|id| evidence.with_broker_stop_order_id(id))
    }
}

fn authoritative_rejection(error: &GrpcError) -> bool {
    matches!(
        error.kind,
        GrpcErrorKind::Provider(ref provider)
            if matches!(
                provider.code,
                Code::InvalidArgument
                    | Code::Unauthenticated
                    | Code::PermissionDenied
                    | Code::NotFound
                    | Code::AlreadyExists
                    | Code::FailedPrecondition
                    | Code::OutOfRange
                    | Code::Unimplemented
            )
    )
}

#[derive(Clone, Debug, Error)]
pub enum ExecutionDispatchError {
    #[error("execution route, authorization and client environments differ")]
    EnvironmentMismatch,
    #[error("logical mutation was already dispatched; authoritative reconciliation required")]
    ReconciliationRequired,
    #[error("invalid execution mutation identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("mutation evidence store failed: {0}")]
    Store(#[from] StoreError),
    #[error("broker authoritatively rejected mutation: {source}")]
    Rejected {
        source: GrpcError,
        evidence: MutationEvidence,
    },
    #[error("mutation outcome unknown after possible dispatch: {source}")]
    UnknownAfterDispatch {
        source: GrpcError,
        evidence: MutationEvidence,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryStore(HashMap<ClientRequestId, MutationEvidence>);

    impl MutationEvidenceStore for MemoryStore {
        fn load(&self, id: &ClientRequestId) -> Result<Option<MutationEvidence>, StoreError> {
            Ok(self.0.get(id).cloned())
        }

        fn persist(&mut self, evidence: &MutationEvidence) -> Result<(), StoreError> {
            self.0
                .insert(evidence.client_request_id().clone(), evidence.clone());
            Ok(())
        }

        fn claim_dispatch(&mut self, evidence: &MutationEvidence) -> Result<bool, StoreError> {
            if self.0.contains_key(evidence.client_request_id()) {
                return Ok(false);
            }
            self.persist(evidence)?;
            Ok(true)
        }

        fn resolve_unknown(
            &mut self,
            expected: &MutationEvidence,
            resolved: &MutationEvidence,
        ) -> Result<bool, StoreError> {
            if self.0.get(expected.client_request_id()) != Some(expected) {
                return Ok(false);
            }
            self.persist(resolved)?;
            Ok(true)
        }
    }

    #[test]
    fn ambiguous_error_codes_are_not_authoritative_rejections() {
        let error = |code| GrpcError {
            metadata: crate::GrpcRequestMetadata {
                request_id: uuid::Uuid::nil(),
                method: "PostOrder",
                attempt: 1,
                mutation: true,
            },
            kind: GrpcErrorKind::Provider(crate::GrpcProviderError {
                code,
                message: String::new(),
                details: vec![],
                tracking_id: None,
            }),
        };
        assert!(!authoritative_rejection(&error(Code::DeadlineExceeded)));
        assert!(!authoritative_rejection(&error(Code::Unavailable)));
        assert!(!authoritative_rejection(&error(Code::Unknown)));
        assert!(authoritative_rejection(&error(Code::InvalidArgument)));
    }

    #[test]
    fn durable_unknown_blocks_duplicate_dispatch() {
        let mut dispatcher = ExecutionMutationDispatcher::new(MemoryStore::default());
        let id = ClientRequestId::new("id").expect("id");
        dispatcher
            .recovery
            .persist_before_dispatch(id.clone(), None)
            .expect("persist unknown");
        assert_eq!(
            dispatcher.decision(&id).expect("decision"),
            MutationDecision::Reconcile
        );
    }
}
