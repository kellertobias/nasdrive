import type { TrustedDeviceProof } from "../api/client";

interface StoredTrustedDevice {
  id: string;
  secret: string;
  hash: string;
}

function storageKey(username: string) {
  return `nasfiles-trusted-totp:${username.trim().toLowerCase()}`;
}

export function storeTrustedTotp(
  username: string,
  device: StoredTrustedDevice,
) {
  localStorage.setItem(storageKey(username), JSON.stringify(device));
}

export function removeTrustedTotp(username: string, id?: string) {
  const stored = loadStored(username);
  if (!stored || (id && stored.id !== id)) return;
  localStorage.removeItem(storageKey(username));
}

// @tour authentication:20 Deriving the trusted-device proof in the browser
// A trusted device is a record in `localStorage` under `nasfiles-trusted-totp:<username>`
// holding `{id, secret, hash}`. This returns the device id, the opaque server-issued hash,
// and a freshly computed code from `generateTotp(stored.secret)`.
//
// `generateTotp` is a hand-rolled RFC 6238 implementation over WebCrypto: HMAC-SHA-1 over a
// 30-second counter, dynamic truncation, modulo 1,000,000. So "remember this computer" is
// literally a second TOTP secret the browser holds on the user's behalf.

export async function trustedTotpProof(
  username: string,
): Promise<TrustedDeviceProof | null> {
  const stored = loadStored(username);
  if (!stored) return null;
  return {
    id: stored.id,
    hash: stored.hash,
    code: await generateTotp(stored.secret),
  };
}

function loadStored(username: string): StoredTrustedDevice | null {
  const raw = localStorage.getItem(storageKey(username));
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as StoredTrustedDevice;
    if (!parsed.id || !parsed.secret || !parsed.hash) return null;
    return parsed;
  } catch {
    return null;
  }
}

async function generateTotp(base32Secret: string): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    toArrayBuffer(decodeBase32(base32Secret)),
    { name: "HMAC", hash: "SHA-1" },
    false,
    ["sign"],
  );
  const counter = Math.floor(Date.now() / 1000 / 30);
  const counterBytes = new ArrayBuffer(8);
  const view = new DataView(counterBytes);
  view.setUint32(4, counter, false);
  const signature = new Uint8Array(
    await crypto.subtle.sign("HMAC", key, counterBytes),
  );
  const offset = signature[signature.length - 1] & 0x0f;
  const value =
    (((signature[offset] & 0x7f) << 24) |
      ((signature[offset + 1] & 0xff) << 16) |
      ((signature[offset + 2] & 0xff) << 8) |
      (signature[offset + 3] & 0xff)) %
    1_000_000;
  return value.toString().padStart(6, "0");
}

function decodeBase32(value: string): Uint8Array {
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  const cleaned = value.toUpperCase().replace(/=+$/g, "").replace(/\s+/g, "");
  let bits = 0;
  let bitCount = 0;
  const out: number[] = [];
  for (const char of cleaned) {
    const index = alphabet.indexOf(char);
    if (index < 0) throw new Error("Invalid trusted TOTP secret");
    bits = (bits << 5) | index;
    bitCount += 5;
    if (bitCount >= 8) {
      out.push((bits >> (bitCount - 8)) & 0xff);
      bitCount -= 8;
    }
  }
  return new Uint8Array(out);
}

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const buffer = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(buffer).set(bytes);
  return buffer;
}
