---
id: ADR-RUNTIME-0012
type: adr
status: ACCEPTED
hardness: H3
owner: Thunder Runtime
team: Edge Platform
domain: Security
created_at: 2026-06-26
---

# ADR 0012: Bundle signing com Ed25519

**Status:** `ACCEPTED (retroativo)`
**Data:** 2026-06-26
**Owner:** Edge Platform
**Knowledge:** `runtime/docs/guides/bundle-signing.md`

## Contexto

Deploy via Admin API precisa de garantia de integridade/autenticidade do bundle para evitar
adulteração em trânsito e criar trilha de auditoria.

## Decisão

Suportar **verificação Ed25519 opcional** no deploy: cliente assina o bundle e envia a assinatura
no header `x-bundle-signature-ed25519` (base64); o runtime valida com a chave pública configurada
(PEM/base64/hex). Assinatura inválida → `400`. Ativado por `--require-bundle-signature`.

## Alternativas consideradas

- **Somente TLS + API key:** protege o canal, mas não a integridade do artefato nem provê trilha.
- **HMAC (chave simétrica):** exige compartilhar segredo; Ed25519 permite chave pública distribuível.

## Consequências

### Positivas
- Detecta adulteração; autoriza apenas partes com a chave privada.

### Negativas / Trade-offs
- Exige gestão de chaves; latência de verificação por deploy.

## Referências
- `runtime/crates/server/src/bundle_signature.rs`
