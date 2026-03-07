# Bundle Signing (Ed25519)

This guide explains how to enable bundle integrity verification during deployment, generate keys, store them securely, rotate them, and operate safely in production.

## Goal

When enabled, the runtime validates an Ed25519 signature for the binary payload sent to:

- `POST /_internal/functions`
- `PUT /_internal/functions/{name}`

If the signature is invalid (or missing when required), deploy/update is rejected.

## How it works

1. The client signs the exact bundle bytes being sent (`.eszip` or `.pkg`) with an Ed25519 private key.
2. The client sends the signature in the `x-bundle-signature-ed25519` header (base64).
3. The runtime verifies it using the configured public key.
4. Only valid bundles are accepted.

Important:

- Validation happens at deploy/update time.
- Already loaded bundles continue running even after key rotation/expiration.

## Enabling in the runtime

`start` flags:

- `--require-bundle-signature`
- `--bundle-public-key-path <PATH>`

Equivalent environment variables:

- `EDGE_RUNTIME_REQUIRE_BUNDLE_SIGNATURE`
- `EDGE_RUNTIME_BUNDLE_PUBLIC_KEY_PATH`

Example:

```bash
cargo run -- start \
  --api-key "admin-secret" \
  --require-bundle-signature \
  --bundle-public-key-path /etc/edge-runtime/keys/bundle-signing-public.pem
```

By default, signature verification is optional (disabled). It becomes mandatory only when
`--require-bundle-signature` (or `EDGE_RUNTIME_REQUIRE_BUNDLE_SIGNATURE=true`) is set.

## Supported public key formats

The runtime accepts `--bundle-public-key-path` content in one of the following formats:

- PEM (`-----BEGIN PUBLIC KEY-----`)
- Base64 raw public key (32 bytes)
- Hex raw public key (32 bytes)

Recommendation: use PEM to reduce operational errors.

## Key generation (OpenSSL)

Prerequisite: OpenSSL 3.x with Ed25519 support.

1. Generate private key:

```bash
openssl genpkey -algorithm ED25519 -out bundle-signing-private.pem
```

2. Extract PEM public key:

```bash
openssl pkey -in bundle-signing-private.pem -pubout -out bundle-signing-public.pem
```

3. Restrict permissions:

```bash
chmod 600 bundle-signing-private.pem
chmod 644 bundle-signing-public.pem
```

## Signing a bundle

Automated option using script:

```bash
./scripts/sign-bundle.sh \
  --bundle ./hello.eszip \
  --private-key ./bundle-signing-private.pem \
  --print-header
```

One-step sign + deploy option:

```bash
./scripts/deploy-signed-bundle.sh \
  --bundle ./hello.eszip \
  --function hello \
  --private-key ./bundle-signing-private.pem \
  --api-key admin-secret
```

For updates, add `--method PUT`.

Manual option with OpenSSL:

Sign the `.eszip` file bytes:

```bash
openssl pkeyutl -sign \
  -inkey bundle-signing-private.pem \
  -rawin \
  -in hello.eszip \
  -out hello.eszip.sig
```

Convert signature to base64 (HTTP header):

```bash
SIG_B64="$(base64 < hello.eszip.sig | tr -d '\n')"
```

Deploy with signature:

```bash
curl -X POST http://127.0.0.1:9000/_internal/functions \
  -H "X-API-Key: admin-secret" \
  -H "x-function-name: hello" \
  -H "x-bundle-signature-ed25519: ${SIG_B64}" \
  --data-binary @hello.eszip
```

Update with signature:

```bash
curl -X PUT http://127.0.0.1:9000/_internal/functions/hello \
  -H "X-API-Key: admin-secret" \
  -H "x-bundle-signature-ed25519: ${SIG_B64}" \
  --data-binary @hello.eszip
```

## Secure key storage

### Private key (signing)

- Never store it in the production runtime.
- Keep it only in the build/release pipeline (secure CI or HSM/KMS).
- Do not commit it to Git.
- Rotate periodically.
- Audit access and usage.

### Public key (verification)

- It can live on the runtime host (read-only).
- Recommended path: `/etc/edge-runtime/keys/` with minimal permissions.
- Manage via configuration management (Ansible, Terraform, etc.).

## Recommended rotation process

1. Generate a new key pair.
2. Update the runtime to use the new public key.
3. Update the pipeline to sign with the new private key.
4. Validate with a canary deploy.
5. Remove the old key from the pipeline.

Operational note:

- Bundles signed with the old key will be rejected after the public key is switched.
- Already deployed functions remain active until a new deploy/restart.

## Expected error responses

When `--require-bundle-signature` is enabled:

- Missing header: `401` with `{"error":"missing x-bundle-signature-ed25519 header"}`
- Invalid header: `401` with `{"error":"invalid x-bundle-signature-ed25519 encoding"}`
- Invalid signature: `401` with `{"error":"bundle signature verification failed"}`

## Additional best practices

- Combine bundle signing with TLS + API key on the admin listener.
- Avoid reusing the same key across different environments (dev/staging/prod).
- Keep secure private key backups with a tested recovery process.
- Maintain an incident runbook for emergency revocation/rotation.
