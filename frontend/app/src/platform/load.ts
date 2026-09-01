import type { ConnectionDetailsDto, ExecutionScope } from "@vox/api-client";
import type { VoxApiError } from "@vox/api-client";
import { browserSession, type BrowserSession, type PlatformAccount, type PlatformSnapshot } from "./types";
import type { VoxService } from "../vox";

export type PlatformLoadResult =
  | { ok: true; value: PlatformSnapshot }
  | { ok: false; error: VoxApiError };

export async function establishAndLoadPlatform(
  service: VoxService,
  bootstrapCredential: string,
): Promise<PlatformLoadResult> {
  const established = await service.establishSession({ bootstrap_credential: bootstrapCredential });
  if (!established.ok) return established;
  return loadPlatform(service, browserSession(established.value));
}

export async function loadPlatform(
  service: VoxService,
  session: BrowserSession,
): Promise<PlatformLoadResult> {
  const [connections, runtime] = await Promise.all([
    service.brokerConnections(),
    service.processRuntime(),
  ]);
  if (!connections.ok) return connections;
  if (!runtime.ok) return runtime;

  const detailResults = await Promise.all(
    connections.value.map((connection) => service.connectionDetails(connection.connection_id)),
  );
  const failed = detailResults.find((result) => !result.ok);
  if (failed !== undefined && !failed.ok) return failed;
  const details = detailResults.flatMap((result) => (result.ok ? [result.value] : []));

  return {
    ok: true,
    value: Object.freeze({
      session,
      connections: Object.freeze([...connections.value]),
      accounts: Object.freeze(details.flatMap(accountsFromDetails)),
      processRuntime: runtime.value,
    }),
  };
}

function accountsFromDetails(details: ConnectionDetailsDto): PlatformAccount[] {
  const connection = details.connection;
  return details.bindings
    .filter((binding) => binding.enabled)
    .map((binding) => {
      const discovered = details.accounts.find(
        (account) => account.provider_account_id === binding.provider_account_id,
      );
      const authorization = details.execution_authorizations.find(
        (item) => item.provider_account_id === binding.provider_account_id,
      );
      const scope: ExecutionScope = {
        provider: binding.provider,
        environment: binding.environment,
        broker_connection_id: binding.connection_id,
        account_id: binding.account_id,
        trading_mode: "LIVE",
      };
      const account: PlatformAccount = {
        scope: Object.freeze(scope),
        connectionLabel: connection.display_label,
        providerAccountId: binding.provider_account_id,
        accountDisplay: discovered?.display_name ?? discovered?.account_type ?? binding.account_id,
        accessible: discovered?.accessible ?? false,
        connectionEnabled: connection.enabled,
        connectionHealth: connection.health,
        connectionCapabilities: Object.freeze([...(discovered?.capabilities ?? connection.capabilities)]),
        binding,
      };
      if (authorization !== undefined) {
        return Object.freeze({ ...account, executionAuthorization: authorization });
      }
      return Object.freeze(account);
    });
}
