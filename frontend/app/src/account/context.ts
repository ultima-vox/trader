import type { ExecutionScope } from "@vox/api-client";

export type AccountContext = Readonly<ExecutionScope>;

export function freezeAccountContext(scope: ExecutionScope): AccountContext {
  return Object.freeze({
    provider: scope.provider,
    environment: scope.environment,
    broker_connection_id: scope.broker_connection_id,
    account_id: scope.account_id,
    trading_mode: scope.trading_mode,
  });
}

export function accountContextKey(ctx: AccountContext): string {
  return [
    ctx.provider,
    ctx.environment,
    ctx.broker_connection_id,
    ctx.account_id,
    ctx.trading_mode,
  ].join("\u001f");
}

export function sameAccountContext(a: AccountContext, b: AccountContext): boolean {
  return accountContextKey(a) === accountContextKey(b);
}
