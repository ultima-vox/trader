import type { ExecutionScope, MutationReceiptDto } from "@vox/api-client";
import { accountContextKey } from "../account/context";

export type FrozenExecutionTarget = Readonly<ExecutionScope> & { readonly frozen: true };

export function freezeExecutionTarget(scope: ExecutionScope): FrozenExecutionTarget {
  return Object.freeze({
    provider: scope.provider,
    environment: scope.environment,
    broker_connection_id: scope.broker_connection_id,
    account_id: scope.account_id,
    trading_mode: scope.trading_mode,
    frozen: true as const,
  });
}

export class CommandHandle {
  readonly scope: FrozenExecutionTarget;
  readonly receipt: MutationReceiptDto | undefined;

  constructor(scope: ExecutionScope, receipt?: MutationReceiptDto) {
    this.scope = freezeExecutionTarget(scope);
    if (receipt !== undefined) {
      assertReceiptMatches(this.scope, receipt);
      this.receipt = freezeReceipt(receipt);
    }
    Object.freeze(this);
  }

  bind(receipt: MutationReceiptDto): CommandHandle {
    return new CommandHandle(this.scope, receipt);
  }
}

export function bindCommand(scope: ExecutionScope, receipt?: MutationReceiptDto): CommandHandle {
  return new CommandHandle(scope, receipt);
}

function assertReceiptMatches(scope: FrozenExecutionTarget, receipt: MutationReceiptDto): void {
  if (accountContextKey(receipt.scope) !== accountContextKey(scope)) {
    throw new Error("receipt scope does not match frozen execution target");
  }
}

function freezeReceipt(receipt: MutationReceiptDto): MutationReceiptDto {
  return Object.freeze({
    ...receipt,
    scope: Object.freeze({ ...receipt.scope }),
  });
}
