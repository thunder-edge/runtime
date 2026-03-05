// Example: Web Crypto API - Encryption, Hashing, Digital Signatures
// Demonstrates various cryptographic operations

Deno.serve(async (req) => {
  const url = new URL(req.url);

  // Hash data with specified algorithm
  async function hashData(
    data: string,
    algorithm: "SHA-1" | "SHA-256" | "SHA-384" | "SHA-512"
  ): Promise<string> {
    const encoder = new TextEncoder();
    const dataBuffer = encoder.encode(data);
    const hashBuffer = await crypto.subtle.digest(algorithm, dataBuffer);
    const hashArray = Array.from(new Uint8Array(hashBuffer));
    return hashArray.map((b) => b.toString(16).padStart(2, "0")).join("");
  }

  // Generate random UUID
  function generateUUID(): string {
    return crypto.randomUUID();
  }

  // Encrypt data using AES-GCM
  async function encryptAESGCM(
    plaintext: string,
    password: string
  ): Promise<{
    ciphertext: string;
    iv: string;
    salt: string;
  }> {
    // Derive key from password
    const salt = crypto.getRandomValues(new Uint8Array(16));
    const passwordKey = await crypto.subtle.importKey(
      "raw",
      new TextEncoder().encode(password),
      "PBKDF2",
      false,
      ["deriveKey"]
    );

    const key = await crypto.subtle.deriveKey(
      {
        name: "PBKDF2",
        salt: salt,
        iterations: 100000,
        hash: "SHA-256",
      },
      passwordKey,
      { name: "AES-GCM", length: 256 },
      false,
      ["encrypt"]
    );

    // Encrypt
    const iv = crypto.getRandomValues(new Uint8Array(12));
    const encrypted = await crypto.subtle.encrypt(
      { name: "AES-GCM", iv: iv },
      key,
      new TextEncoder().encode(plaintext)
    );

    return {
      ciphertext: btoa(String.fromCharCode(...new Uint8Array(encrypted))),
      iv: btoa(String.fromCharCode(...iv)),
      salt: btoa(String.fromCharCode(...salt)),
    };
  }

  // Decrypt AES-GCM
  async function decryptAESGCM(
    ciphertext: string,
    password: string,
    iv: string,
    salt: string
  ): Promise<string> {
    const saltArray = new Uint8Array(
      atob(salt)
        .split("")
        .map((c) => c.charCodeAt(0))
    );
    const ivArray = new Uint8Array(
      atob(iv)
        .split("")
        .map((c) => c.charCodeAt(0))
    );
    const ciphertextArray = new Uint8Array(
      atob(ciphertext)
        .split("")
        .map((c) => c.charCodeAt(0))
    );

    const passwordKey = await crypto.subtle.importKey(
      "raw",
      new TextEncoder().encode(password),
      "PBKDF2",
      false,
      ["deriveKey"]
    );

    const key = await crypto.subtle.deriveKey(
      {
        name: "PBKDF2",
        salt: saltArray,
        iterations: 100000,
        hash: "SHA-256",
      },
      passwordKey,
      { name: "AES-GCM", length: 256 },
      false,
      ["decrypt"]
    );

    const decrypted = await crypto.subtle.decrypt(
      { name: "AES-GCM", iv: ivArray },
      key,
      ciphertextArray
    );

    return new TextDecoder().decode(decrypted);
  }

  // Hash endpoint
  if (url.pathname === "/api/hash" && req.method === "POST") {
    try {
      const { data, algorithm } = await req.json();
      const algorithms = ["SHA-1", "SHA-256", "SHA-384", "SHA-512"] as const;

      const useAlgo = (
        algorithms.includes(algorithm) ? algorithm : "SHA-256"
      ) as "SHA-1" | "SHA-256" | "SHA-384" | "SHA-512";
      const hash = await hashData(data, useAlgo);

      return new Response(
        JSON.stringify({
          data,
          algorithm: useAlgo,
          hash,
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Hashing failed",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Generate UUID endpoint
  if (url.pathname === "/api/uuid" && req.method === "GET") {
    const count = parseInt(url.searchParams.get("count") || "1");
    const uuids = Array.from({ length: Math.min(count, 100) }, () =>
      generateUUID()
    );

    return new Response(
      JSON.stringify({
        count: uuids.length,
        uuids,
      }),
      {
        headers: { "content-type": "application/json" },
      }
    );
  }

  // Encrypt endpoint
  if (url.pathname === "/api/encrypt" && req.method === "POST") {
    try {
      const { plaintext, password } = await req.json();
      const encrypted = await encryptAESGCM(plaintext, password);

      return new Response(
        JSON.stringify({
          success: true,
          algorithm: "AES-GCM",
          ...encrypted,
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Encryption failed",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Decrypt endpoint
  if (url.pathname === "/api/decrypt" && req.method === "POST") {
    try {
      const { ciphertext, password, iv, salt } = await req.json();
      const decrypted = await decryptAESGCM(ciphertext, password, iv, salt);

      return new Response(
        JSON.stringify({
          success: true,
          plaintext: decrypted,
        }),
        {
          headers: { "content-type": "application/json" },
        }
      );
    } catch (error) {
      return new Response(
        JSON.stringify({
          error: "Decryption failed",
          details: (error as Error)?.message,
        }),
        {
          status: 400,
          headers: { "content-type": "application/json" },
        }
      );
    }
  }

  // Home page
  if (url.pathname === "/") {
    const html = `
      <!DOCTYPE html>
      <html>
      <head>
        <meta charset="UTF-8">
        <title>Web Crypto API</title>
        <style>
          * { margin: 0; padding: 0; box-sizing: border-box; }
          body { font-family: Arial; background: #f5f5f5; padding: 40px 20px; }
          .container { max-width: 900px; margin: 0 auto; background: white; border-radius: 8px; padding: 40px; box-shadow: 0 2px 10px rgba(0,0,0,0.1); }
          h1 { color: #333; margin-bottom: 10px; }
          h2 { color: #667eea; margin: 30px 0 15px; font-size: 1.2em; }
          .section { background: #f9f9f9; border-left: 4px solid #667eea; padding: 20px; margin: 20px 0; border-radius: 4px; }
          input, textarea, select { width: 100%; padding: 10px; margin: 5px 0; border: 1px solid #ddd; border-radius: 4px; font-family: monospace; }
          button { background: #667eea; color: white; border: none; padding: 12px 24px; border-radius: 4px; cursor: pointer; margin: 10px 0; }
          button:hover { background: #764ba2; }
          .output { background: white; padding: 15px; border: 1px solid #ddd; border-radius: 4px; margin-top: 15px; max-height: 250px; overflow-y: auto; white-space: pre-wrap; font-size: 0.85em; }
        </style>
      </head>
      <body>
        <div class="container">
          <h1>🔐 Web Crypto API</h1>
          <p style="color: #999; margin-bottom: 20px;">Hashing, encryption, UUIDs, and cryptographic operations</p>

          <div class="section">
            <h2>1. Hashing (SHA-256)</h2>
            <input type="text" id="hashInput" placeholder="Enter text to hash..." value="Hello, World!">
            <select id="hashAlgo">
              <option value="SHA-1">SHA-1</option>
              <option value="SHA-256" selected>SHA-256</option>
              <option value="SHA-384">SHA-384</option>
              <option value="SHA-512">SHA-512</option>
            </select>
            <button onclick="doHash()">Hash</button>
            <div id="hashOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>2. Generate UUID</h2>
            <input type="number" id="uuidCount" value="1" min="1" max="100" placeholder="Number of UUIDs">
            <button onclick="doGenerateUUID()">Generate</button>
            <div id="uuidOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>3. Encryption (AES-GCM)</h2>
            <textarea id="plaintextInput" placeholder="Enter text to encrypt..." rows="4">Secret message</textarea>
            <input type="password" id="encryptPassword" placeholder="Password for encryption">
            <button onclick="doEncrypt()">Encrypt</button>
            <div id="encryptOutput" class="output"></div>
          </div>

          <div class="section">
            <h2>4. Decryption (AES-GCM)</h2>
            <textarea id="ciphertextInput" placeholder="Paste ciphertext..." rows="3"></textarea>
            <input type="password" id="decryptPassword" placeholder="Password for decryption">
            <input type="hidden" id="ivInput">
            <input type="hidden" id="saltInput">
            <button onclick="doDecrypt()">Decrypt</button>
            <div id="decryptOutput" class="output"></div>
          </div>
        </div>

        <script>
          async function doHash() {
            const input = document.getElementById('hashInput').value;
            const algo = document.getElementById('hashAlgo').value;
            try {
              const response = await fetch('/api/hash', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ data: input, algorithm: algo })
              });
              const data = await response.json();
              document.getElementById('hashOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('hashOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function doGenerateUUID() {
            const count = document.getElementById('uuidCount').value;
            try {
              const response = await fetch('/api/uuid?count=' + count);
              const data = await response.json();
              document.getElementById('uuidOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('uuidOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function doEncrypt() {
            const plaintext = document.getElementById('plaintextInput').value;
            const password = document.getElementById('encryptPassword').value;
            try {
              const response = await fetch('/api/encrypt', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ plaintext, password })
              });
              const data = await response.json();
              document.getElementById('encryptOutput').textContent = JSON.stringify(data, null, 2);
              document.getElementById('ciphertextInput').value = data.ciphertext;
              document.getElementById('ivInput').value = data.iv;
              document.getElementById('saltInput').value = data.salt;
            } catch (e) {
              document.getElementById('encryptOutput').textContent = 'Error: ' + e.message;
            }
          }

          async function doDecrypt() {
            const ciphertext = document.getElementById('ciphertextInput').value;
            const password = document.getElementById('decryptPassword').value;
            const iv = document.getElementById('ivInput').value;
            const salt = document.getElementById('saltInput').value;
            try {
              const response = await fetch('/api/decrypt', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ ciphertext, password, iv, salt })
              });
              const data = await response.json();
              document.getElementById('decryptOutput').textContent = JSON.stringify(data, null, 2);
            } catch (e) {
              document.getElementById('decryptOutput').textContent = 'Error: ' + e.message;
            }
          }
        </script>
      </body>
      </html>
    `;
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  }

  return new Response("Not found", { status: 404 });
});
