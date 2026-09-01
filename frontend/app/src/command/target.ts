import type { ExecutionScope, MutationReceiptDto } from "@vox/api-client";
import { accountContextKey } from "../account/context";

export type FrozenExecutionTarget = Readonly<ExecutionScope> & { readonly frozen: true };

export type CommandTargetDisplay = Readonly<{
  providerAccountId: string;
  accountDisplay: string;
  connectionLabel: string;
}>;

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
  readonly logicalRequestId: string;
  readonly receipt: MutationReceiptDto | undefined;
  readonly targetDisplay: CommandTargetDisplay | undefined;

  constructor(
    scope: ExecutionScope,
    logicalRequestId = newLogicalRequestId(),
    receipt?: MutationReceiptDto,
    targetDisplay?: CommandTargetDisplay,
  ) {
    this.scope = freezeExecutionTarget(scope);
    this.logicalRequestId = logicalRequestId;
    if (receipt !== undefined) {
      assertReceiptMatches(this.scope, this.logicalRequestId, receipt);
      this.receipt = freezeReceipt(receipt);
    }
    if (targetDisplay !== undefined) this.targetDisplay = Object.freeze({ ...targetDisplay });
    Object.freeze(this);
  }

  bind(receipt: MutationReceiptDto): CommandHandle {
    return new CommandHandle(this.scope, this.logicalRequestId, receipt, this.targetDisplay);
  }
}

export function bindCommand(
  scope: ExecutionScope,
  logicalRequestId = newLogicalRequestId(),
  receipt?: MutationReceiptDto,
  targetDisplay?: CommandTargetDisplay,
): CommandHandle {
  return new CommandHandle(scope, logicalRequestId, receipt, targetDisplay);
}

function assertReceiptMatches(
  scope: FrozenExecutionTarget,
  logicalRequestId: string,
  receipt: MutationReceiptDto,
): void {
  if (accountContextKey(receipt.scope) !== accountContextKey(scope)) {
    throw new Error("receipt scope does not match frozen execution target");
  }
  if (receipt.logical_request_id !== logicalRequestId) {
    throw new Error("receipt identity does not match frozen logical request");
  }
}

function newLogicalRequestId(): string {
  return globalThis.crypto.randomUUID();
}

function freezeReceipt(receipt: MutationReceiptDto): MutationReceiptDto {
  return Object.freeze({
    ...receipt,
    scope: Object.freeze({ ...receipt.scope }),
  });
}
