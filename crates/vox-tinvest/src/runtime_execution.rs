//! Stateless typed runtime adapter. Runtime owns durability and UNKNOWN fencing;
//! this adapter performs exactly one provider mutation transport attempt.

use thiserror::Error;
use tonic::Code;
use vox_domain::{
    Environment, LiveMutationError, MutationGuard, ProviderOrderIdentityKind,
    RuntimeExecutionCommand,
};

use crate::execution::{
    ExecutionValidationError, cancel_order_request, cancel_stop_order_request,
    protection_leg_request, regular_order_request, replace_order_request,
};
use crate::execution_dispatch::ExecutionRoute;
use crate::{GrpcError, GrpcErrorKind, TInvestGrpcClient};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeDispatchAcknowledgement {
    pub transport_request_id: String,
    pub broker_order_id: Option<String>,
    pub replacement_broker_order_id: Option<String>,
    pub broker_stop_order_id: Option<String>,
    pub provider_operation_id: Option<String>,
}

pub struct TInvestRuntimeExecutionAdapter {
    client: TInvestGrpcClient,
    route: ExecutionRoute,
    live_mutations_enabled: bool,
}

impl TInvestRuntimeExecutionAdapter {
    #[must_use]
    pub const fn new(
        client: TInvestGrpcClient,
        route: ExecutionRoute,
        live_mutations_enabled: bool,
    ) -> Self {
        Self {
            client,
            route,
            live_mutations_enabled,
        }
    }

    pub async fn dispatch_once(
        &self,
        command: &RuntimeExecutionCommand,
    ) -> Result<RuntimeDispatchAcknowledgement, RuntimeExecutionAdapterError> {
        let authorization = self.authorization()?;
        match command {
            RuntimeExecutionCommand::RegularOrder(command) => {
                let request = regular_order_request(command)?;
                let response = match self.route {
                    ExecutionRoute::Production => {
                        self.client.post_order(authorization, request).await?
                    }
                    ExecutionRoute::Sandbox => {
                        self.client
                            .post_sandbox_order(authorization, request)
                            .await?
                    }
                };
                Ok(RuntimeDispatchAcknowledgement {
                    transport_request_id: response.metadata.request_id.to_string(),
                    broker_order_id: nonempty(response.body.order_id),
                    replacement_broker_order_id: None,
                    broker_stop_order_id: None,
                    provider_operation_id: None,
                })
            }
            RuntimeExecutionCommand::PostOrderAsync(command) => {
                let request = crate::execution::async_regular_order_request(command)?;
                let response = match self.route {
                    ExecutionRoute::Production => {
                        self.client.post_order_async(authorization, request).await?
                    }
                    ExecutionRoute::Sandbox => {
                        self.client
                            .post_sandbox_order_async(authorization, request)
                            .await?
                    }
                };
                Ok(RuntimeDispatchAcknowledgement {
                    transport_request_id: response.metadata.request_id.to_string(),
                    broker_order_id: None,
                    replacement_broker_order_id: None,
                    broker_stop_order_id: None,
                    provider_operation_id: response
                        .body
                        .trade_intent_id
                        .filter(|value| nonblank(value)),
                })
            }
            RuntimeExecutionCommand::ReplaceOrder(command) => {
                let request = replace_order_request(command)?;
                let response = match self.route {
                    ExecutionRoute::Production => {
                        self.client.replace_order(authorization, request).await?
                    }
                    ExecutionRoute::Sandbox => {
                        self.client
                            .replace_sandbox_order(authorization, request)
                            .await?
                    }
                };
                Ok(RuntimeDispatchAcknowledgement {
                    transport_request_id: response.metadata.request_id.to_string(),
                    broker_order_id: None,
                    replacement_broker_order_id: nonempty(response.body.order_id),
                    broker_stop_order_id: None,
                    provider_operation_id: None,
                })
            }
            RuntimeExecutionCommand::CancelOrder(command) => {
                let request = cancel_order_request(command)?;
                let response = match self.route {
                    ExecutionRoute::Production => {
                        self.client.cancel_order(authorization, request).await?
                    }
                    ExecutionRoute::Sandbox => {
                        self.client
                            .cancel_sandbox_order(authorization, request)
                            .await?
                    }
                };
                Ok(RuntimeDispatchAcknowledgement {
                    transport_request_id: response.metadata.request_id.to_string(),
                    broker_order_id: matches!(
                        command.order_id_kind,
                        None | Some(ProviderOrderIdentityKind::BrokerOrder)
                    )
                    .then(|| command.order_id.clone()),
                    replacement_broker_order_id: None,
                    broker_stop_order_id: None,
                    provider_operation_id: None,
                })
            }
            RuntimeExecutionCommand::PostStopOrder(command)
            | RuntimeExecutionCommand::ProtectionLeg(command) => {
                let request = protection_leg_request(command)?;
                let response = match self.route {
                    ExecutionRoute::Production => {
                        self.client.post_stop_order(authorization, request).await?
                    }
                    ExecutionRoute::Sandbox => {
                        self.client
                            .post_sandbox_stop_order(authorization, request)
                            .await?
                    }
                };
                Ok(RuntimeDispatchAcknowledgement {
                    transport_request_id: response.metadata.request_id.to_string(),
                    broker_order_id: None,
                    replacement_broker_order_id: None,
                    broker_stop_order_id: nonempty(response.body.stop_order_id),
                    provider_operation_id: nonempty(response.body.order_request_id),
                })
            }
            RuntimeExecutionCommand::CancelStopOrder(command) => {
                let request = cancel_stop_order_request(command)?;
                let response = match self.route {
                    ExecutionRoute::Production => {
                        self.client
                            .cancel_stop_order(authorization, request)
                            .await?
                    }
                    ExecutionRoute::Sandbox => {
                        self.client
                            .cancel_sandbox_stop_order(authorization, request)
                            .await?
                    }
                };
                Ok(RuntimeDispatchAcknowledgement {
                    transport_request_id: response.metadata.request_id.to_string(),
                    broker_order_id: None,
                    replacement_broker_order_id: None,
                    broker_stop_order_id: Some(command.broker_stop_order_id.clone()),
                    provider_operation_id: None,
                })
            }
        }
    }

    fn authorization(&self) -> Result<vox_domain::MutationAuthorization, LiveMutationError> {
        let environment = match self.route {
            ExecutionRoute::Production => Environment::Live,
            ExecutionRoute::Sandbox => Environment::Sandbox,
        };
        let guard = if self.live_mutations_enabled {
            MutationGuard::with_live_mutations_enabled(environment)
        } else {
            MutationGuard::new(environment)
        };
        guard.authorize_mutation()
    }
}

#[must_use]
pub fn authoritative_rejection(error: &GrpcError) -> bool {
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

fn nonempty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn nonblank(value: &str) -> bool {
    !value.trim().is_empty()
}

#[derive(Debug, Error)]
pub enum RuntimeExecutionAdapterError {
    #[error(transparent)]
    Validation(#[from] ExecutionValidationError),
    #[error(transparent)]
    Authorization(#[from] LiveMutationError),
    #[error(transparent)]
    Transport(#[from] GrpcError),
}
