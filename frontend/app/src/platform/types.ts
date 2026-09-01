import type {
  AuthSessionDto,
  BrokerAccountBindingDto,
  BrokerConnectionMetadataDto,
  ConnectionCapabilityDto,
  ConnectionHealthDto,
  ExecutionAuthorizationDto,
  ExecutionScope,
  PermissionDto,
  RuntimeHealthDto,
} from "@vox/api-client";

export type BrowserSession = Readonly<{
  userId: string;
  effectivePermissions: ReadonlySet<PermissionDto>;
  expiresAtUnixMs: number;
  csrfReady: boolean;
}>;

export type PlatformAccount = Readonly<{
  scope: ExecutionScope;
  connectionLabel: string;
  providerAccountId: string;
  accountDisplay: string;
  accessible: boolean;
  connectionEnabled: boolean;
  connectionHealth: ConnectionHealthDto;
  connectionCapabilities: readonly ConnectionCapabilityDto[];
  binding: BrokerAccountBindingDto;
  executionAuthorization?: ExecutionAuthorizationDto;
}>;

export type PlatformSnapshot = Readonly<{
  session: BrowserSession;
  connections: readonly BrokerConnectionMetadataDto[];
  accounts: readonly PlatformAccount[];
  processRuntime: RuntimeHealthDto;
}>;

export function browserSession(dto: AuthSessionDto): BrowserSession {
  return Object.freeze({
    userId: dto.user_id,
    effectivePermissions: new Set(dto.effective_permissions),
    expiresAtUnixMs: dto.expires_at_unix_ms,
    csrfReady: dto.csrf_token.length > 0,
  });
}
