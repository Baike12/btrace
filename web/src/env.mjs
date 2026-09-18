/**
 * PG-Only simplified env validation.
 * Removed ~100 SSO/OAuth/SMTP/ClickHouse/Redis vars.
 * Only validates config actually used by the PG-only frontend.
 */
import { z } from "zod";
import { createEnv } from "@t3-oss/env-nextjs";

export const env = createEnv({
  server: {
    // Database
    DATABASE_URL: z.string().url(),
    NODE_ENV: z.enum(["development", "test", "production"]),
    BUILD_ID: z.string().optional(),

    // Auth (JWT secret used by Rust backend, referenced by Next.js)
    NEXTAUTH_SECRET: z.string().min(1).optional(),
    NEXTAUTH_URL: z.string().optional(),
    SALT: z.string({
      error: () =>
        "A strong SALT is required to encrypt API keys securely.",
    }).optional(),

    // Init / auto-provisioning
    LANGFUSE_INIT_ORG_ID: z.string().optional(),
    LANGFUSE_INIT_ORG_NAME: z.string().optional(),
    LANGFUSE_INIT_ORG_CLOUD_PLAN: z.string().optional(),
    LANGFUSE_INIT_PROJECT_ID: z.string().optional(),
    LANGFUSE_INIT_PROJECT_NAME: z.string().optional(),
    LANGFUSE_INIT_PROJECT_RETENTION: z.coerce.number().optional(),
    LANGFUSE_INIT_PROJECT_PUBLIC_KEY: z.string().optional(),
    LANGFUSE_INIT_PROJECT_SECRET_KEY: z.string().optional(),
    LANGFUSE_INIT_USER_EMAIL: z.string().optional(),
    LANGFUSE_INIT_USER_NAME: z.string().optional(),
    LANGFUSE_INIT_USER_PASSWORD: z.string().optional(),
    // Comma-separated existing accounts to bind to the provisioned org/project
    // as OWNER. Without it the auto-created org has no members and ingested
    // data is unreachable in the browser.
    LANGFUSE_INIT_ORG_MEMBER_EMAIL: z.string().optional(),

    // Default org/project on signup
    LANGFUSE_DEFAULT_ORG_ID: z.string().optional(),
    LANGFUSE_DEFAULT_ORG_ROLE: z.string().optional(),
    LANGFUSE_DEFAULT_PROJECT_ID: z.string().optional(),
    LANGFUSE_DEFAULT_PROJECT_ROLE: z.string().optional(),

    // Rust backend URL
    RUST_API_URL: z.string().optional(),

    // ---- Rust backend: session, signup and CORS ----
    // Signing key for the console's session cookie. The Rust process verifies
    // with this value and the Next.js process must not need it, but declaring
    // it here documents that both halves run with the same environment.
    JWT_SECRET: z.string().optional(),
    // Set `true` to close `POST /api/auth/signup` on instances whose accounts
    // are provisioned with `LANGFUSE_INIT_*` instead.
    LANGFUSE_DISABLE_SIGNUP: z.enum(["true", "false"]).optional(),
    // Set `true` when the deployment terminates TLS, so the session cookie
    // carries `Secure`. Leave unset for plain-HTTP self-hosting: a `Secure`
    // cookie is dropped over HTTP and sign-in silently fails.
    LANGFUSE_SECURE_COOKIES: z.enum(["true", "false"]).optional(),
    // Comma-separated origins allowed to call the API cross-origin. Unset means
    // no cross-origin access, which is what the shipped same-origin topology
    // needs.
    LANGFUSE_CORS_ALLOWED_ORIGINS: z.string().optional(),

    // Telemetry
    TELEMETRY_ENABLED: z.enum(["true", "false"]).optional(),

    // Encryption
    ENCRYPTION_KEY: z.string().optional(),

    // Email (optional, for password reset etc.)
    EMAIL_FROM_ADDRESS: z.string().optional(),
    SMTP_CONNECTION_URL: z.string().optional(),

    // License
    LANGFUSE_EE_LICENSE_KEY: z.string().optional(),

    // Misc
    LANGFUSE_CSP_ENFORCE_HTTPS: z.enum(["true", "false"]).optional(),
    LANGFUSE_ENABLE_EXPERIMENTAL_FEATURES: z.enum(["true", "false"]).optional(),
    LANGFUSE_MIGRATION_V4_ALLOW_PREVIEW_OPT_IN: z.enum(["true", "false"]).optional(),
    SEED_SECRET_KEY: z.string().min(1).optional(),
    LANGFUSE_MCP_ALLOWED_HOSTS: z.string().optional(),

    // OTEL
    OTEL_EXPORTER_OTLP_ENDPOINT: z.string().default("http://localhost:4318"),
    OTEL_SERVICE_NAME: z.string().default("web"),
    OTEL_TRACE_SAMPLING_RATIO: z.coerce.number().gte(0).lte(1).default(1),

    // Others (allow string passthrough for legacy compat)
    PLAIN_AUTHENTICATION_SECRET: z.string().optional(),
    STRIPE_SECRET_KEY: z.string().optional(),
    SENTRY_AUTH_TOKEN: z.string().optional(),
    SENTRY_ORG: z.string().optional(),
    SENTRY_PROJECT: z.string().optional(),
    SENTRY_CSP_REPORT_URI: z.string().optional(),
    AUTH_EMAIL_VERIFICATION_REQUIRED: z.enum(["true", "false"]).optional(),
    NEXT_MANUAL_SIG_HANDLE: z.string().optional(),
    DOCKER_BUILD: z.string().optional(),
  },

  client: {
    NEXT_PUBLIC_BASE_PATH: z.string().optional(),
    NEXT_PUBLIC_LANGFUSE_CLOUD_REGION: z.string().optional(),
    NEXT_PUBLIC_LANGFUSE_RUN_NEXT_INIT: z.enum(["true", "false"]).optional(),
    NEXT_PUBLIC_POSTHOG_KEY: z.string().optional(),
    NEXT_PUBLIC_POSTHOG_HOST: z.string().optional(),
    NEXT_PUBLIC_DEMO_ORG_ID: z.string().optional(),
    NEXT_PUBLIC_DEMO_PROJECT_ID: z.string().optional(),
    NEXT_PUBLIC_LANGFUSE_PLAYGROUND_STREAMING_ENABLED_DEFAULT: z.enum(["true", "false"]).optional(),
    NEXT_PUBLIC_BUILD_ID: z.string().optional(),
    NEXT_PUBLIC_API_URL: z.string().optional(),
  },

  runtimeEnv: {
    DATABASE_URL: process.env.DATABASE_URL,
    NODE_ENV: process.env.NODE_ENV,
    BUILD_ID: process.env.BUILD_ID,
    NEXTAUTH_SECRET: process.env.NEXTAUTH_SECRET,
    NEXTAUTH_URL: process.env.NEXTAUTH_URL,
    SALT: process.env.SALT,
    LANGFUSE_INIT_ORG_ID: process.env.LANGFUSE_INIT_ORG_ID,
    LANGFUSE_INIT_ORG_NAME: process.env.LANGFUSE_INIT_ORG_NAME,
    LANGFUSE_INIT_ORG_CLOUD_PLAN: process.env.LANGFUSE_INIT_ORG_CLOUD_PLAN,
    LANGFUSE_INIT_PROJECT_ID: process.env.LANGFUSE_INIT_PROJECT_ID,
    LANGFUSE_INIT_PROJECT_NAME: process.env.LANGFUSE_INIT_PROJECT_NAME,
    LANGFUSE_INIT_PROJECT_RETENTION: process.env.LANGFUSE_INIT_PROJECT_RETENTION,
    LANGFUSE_INIT_PROJECT_PUBLIC_KEY: process.env.LANGFUSE_INIT_PROJECT_PUBLIC_KEY,
    LANGFUSE_INIT_PROJECT_SECRET_KEY: process.env.LANGFUSE_INIT_PROJECT_SECRET_KEY,
    LANGFUSE_INIT_USER_EMAIL: process.env.LANGFUSE_INIT_USER_EMAIL,
    LANGFUSE_INIT_USER_NAME: process.env.LANGFUSE_INIT_USER_NAME,
    LANGFUSE_INIT_USER_PASSWORD: process.env.LANGFUSE_INIT_USER_PASSWORD,
    LANGFUSE_INIT_ORG_MEMBER_EMAIL: process.env.LANGFUSE_INIT_ORG_MEMBER_EMAIL,
    LANGFUSE_DEFAULT_ORG_ID: process.env.LANGFUSE_DEFAULT_ORG_ID,
    LANGFUSE_DEFAULT_ORG_ROLE: process.env.LANGFUSE_DEFAULT_ORG_ROLE,
    LANGFUSE_DEFAULT_PROJECT_ID: process.env.LANGFUSE_DEFAULT_PROJECT_ID,
    LANGFUSE_DEFAULT_PROJECT_ROLE: process.env.LANGFUSE_DEFAULT_PROJECT_ROLE,
    RUST_API_URL: process.env.RUST_API_URL,
    JWT_SECRET: process.env.JWT_SECRET,
    LANGFUSE_DISABLE_SIGNUP: process.env.LANGFUSE_DISABLE_SIGNUP,
    LANGFUSE_SECURE_COOKIES: process.env.LANGFUSE_SECURE_COOKIES,
    LANGFUSE_CORS_ALLOWED_ORIGINS: process.env.LANGFUSE_CORS_ALLOWED_ORIGINS,
    TELEMETRY_ENABLED: process.env.TELEMETRY_ENABLED,
    ENCRYPTION_KEY: process.env.ENCRYPTION_KEY,
    EMAIL_FROM_ADDRESS: process.env.EMAIL_FROM_ADDRESS,
    SMTP_CONNECTION_URL: process.env.SMTP_CONNECTION_URL,
    LANGFUSE_EE_LICENSE_KEY: process.env.LANGFUSE_EE_LICENSE_KEY,
    LANGFUSE_CSP_ENFORCE_HTTPS: process.env.LANGFUSE_CSP_ENFORCE_HTTPS,
    LANGFUSE_ENABLE_EXPERIMENTAL_FEATURES: process.env.LANGFUSE_ENABLE_EXPERIMENTAL_FEATURES,
    LANGFUSE_MIGRATION_V4_ALLOW_PREVIEW_OPT_IN: process.env.LANGFUSE_MIGRATION_V4_ALLOW_PREVIEW_OPT_IN,
    SEED_SECRET_KEY: process.env.SEED_SECRET_KEY,
    LANGFUSE_MCP_ALLOWED_HOSTS: process.env.LANGFUSE_MCP_ALLOWED_HOSTS,
    OTEL_EXPORTER_OTLP_ENDPOINT: process.env.OTEL_EXPORTER_OTLP_ENDPOINT,
    OTEL_SERVICE_NAME: process.env.OTEL_SERVICE_NAME,
    OTEL_TRACE_SAMPLING_RATIO: process.env.OTEL_TRACE_SAMPLING_RATIO,
    PLAIN_AUTHENTICATION_SECRET: process.env.PLAIN_AUTHENTICATION_SECRET,
    STRIPE_SECRET_KEY: process.env.STRIPE_SECRET_KEY,
    SENTRY_AUTH_TOKEN: process.env.SENTRY_AUTH_TOKEN,
    SENTRY_ORG: process.env.SENTRY_ORG,
    SENTRY_PROJECT: process.env.SENTRY_PROJECT,
    SENTRY_CSP_REPORT_URI: process.env.SENTRY_CSP_REPORT_URI,
    AUTH_EMAIL_VERIFICATION_REQUIRED: process.env.AUTH_EMAIL_VERIFICATION_REQUIRED,
    NEXT_MANUAL_SIG_HANDLE: process.env.NEXT_MANUAL_SIG_HANDLE,
    DOCKER_BUILD: process.env.DOCKER_BUILD,
    NEXT_PUBLIC_BASE_PATH: process.env.NEXT_PUBLIC_BASE_PATH,
    NEXT_PUBLIC_LANGFUSE_CLOUD_REGION: process.env.NEXT_PUBLIC_LANGFUSE_CLOUD_REGION,
    NEXT_PUBLIC_LANGFUSE_RUN_NEXT_INIT: process.env.NEXT_PUBLIC_LANGFUSE_RUN_NEXT_INIT,
    NEXT_PUBLIC_POSTHOG_KEY: process.env.NEXT_PUBLIC_POSTHOG_KEY,
    NEXT_PUBLIC_POSTHOG_HOST: process.env.NEXT_PUBLIC_POSTHOG_HOST,
    NEXT_PUBLIC_DEMO_ORG_ID: process.env.NEXT_PUBLIC_DEMO_ORG_ID,
    NEXT_PUBLIC_DEMO_PROJECT_ID: process.env.NEXT_PUBLIC_DEMO_PROJECT_ID,
    NEXT_PUBLIC_LANGFUSE_PLAYGROUND_STREAMING_ENABLED_DEFAULT: process.env.NEXT_PUBLIC_LANGFUSE_PLAYGROUND_STREAMING_ENABLED_DEFAULT,
    NEXT_PUBLIC_BUILD_ID: process.env.NEXT_PUBLIC_BUILD_ID,
    NEXT_PUBLIC_API_URL: process.env.NEXT_PUBLIC_API_URL,
  },

  skipValidation: process.env.DOCKER_BUILD === "1",
});
