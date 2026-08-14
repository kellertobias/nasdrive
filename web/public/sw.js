// Bumped with the caching-strategy change so the activate handler drops
// entries written under the old, broader interception rules.
const CACHE_NAME = "nasfiles-shell-v3";
const SHELL_ASSETS = ["/", "/manifest.webmanifest", "/favicon.svg"];
const SHARE_DB_NAME = "nasfiles-share-target";
const SHARE_STORE_NAME = "incoming-shares";

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE_NAME)
      .then((cache) => cache.addAll(SHELL_ASSETS))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys
            .filter((key) => key !== CACHE_NAME)
            .map((key) => caches.delete(key)),
        ),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const url = new URL(event.request.url);

  if (
    event.request.method === "POST" &&
    url.origin === self.location.origin &&
    url.pathname === "/share-target"
  ) {
    event.respondWith(handleShareTarget(event.request));
    return;
  }

  if (event.request.method !== "GET" || url.origin !== self.location.origin) {
    return;
  }

  if (event.request.mode === "navigate") {
    event.respondWith(navigationFallback(event.request));
    return;
  }

  if (isStaticAsset(url.pathname)) {
    event.respondWith(
      cacheFirst(event.request, { revalidate: !isImmutableAsset(url.pathname) }),
    );
    return;
  }

  // Everything else — API calls, thumbnails, downloads, media streams — falls
  // through to the network untouched. Calling respondWith here would put a
  // CacheStorage lookup in front of every request for entries this worker never
  // writes, and would route file downloads and ranged media through the worker,
  // which defeats streaming and makes the browser buffer large files in memory.
});

async function handleShareTarget(request) {
  const formData = await request.formData();
  const files = formData.getAll("files").filter((file) => file instanceof File);
  const title = stringifyFormValue(formData.get("title"));
  const text = stringifyFormValue(formData.get("text"));
  const url = stringifyFormValue(formData.get("url"));
  const id = `${Date.now()}-${crypto.randomUUID()}`;

  await storeShare({
    id,
    title,
    text,
    url,
    createdAt: Date.now(),
    files: files.map((file) => ({
      name: file.name || "shared-file",
      type: file.type || "application/octet-stream",
      lastModified: file.lastModified || Date.now(),
      blob: file,
    })),
  });

  return Response.redirect(`/share-target?shareId=${encodeURIComponent(id)}`, 303);
}

function stringifyFormValue(value) {
  return typeof value === "string" ? value : "";
}

function openShareDb() {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(SHARE_DB_NAME, 1);
    request.onupgradeneeded = () => {
      request.result.createObjectStore(SHARE_STORE_NAME, { keyPath: "id" });
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function storeShare(record) {
  const db = await openShareDb();
  await new Promise((resolve, reject) => {
    const tx = db.transaction(SHARE_STORE_NAME, "readwrite");
    tx.objectStore(SHARE_STORE_NAME).put(record);
    tx.oncomplete = () => resolve();
    tx.onerror = () => reject(tx.error);
  });
  db.close();
}

function isStaticAsset(pathname) {
  return (
    isImmutableAsset(pathname) ||
    pathname.startsWith("/pwa/") ||
    pathname === "/manifest.webmanifest" ||
    pathname === "/favicon.svg"
  );
}

/**
 * Vite writes content-hashed filenames under /assets/, so a cached response
 * there can never go stale — a changed file arrives under a new name.
 */
function isImmutableAsset(pathname) {
  return pathname.startsWith("/assets/");
}

/**
 * Serve from cache, filling it on a miss.
 *
 * `revalidate` refreshes the entry in the background after serving, for assets
 * whose filename stays the same across builds. Immutable assets skip it: doing
 * it there meant re-downloading the whole JS bundle on every single load to
 * replace it with a byte-identical copy.
 */
async function cacheFirst(request, { revalidate }) {
  const cached = await caches.match(request);

  const fromNetwork = async () => {
    const response = await fetch(request);
    if (response.ok) {
      const cache = await caches.open(CACHE_NAME);
      await cache.put(request, response.clone());
    }
    return response;
  };

  if (!cached) return fromNetwork();
  if (revalidate) fromNetwork().catch(() => undefined);
  return cached;
}

async function navigationFallback(request) {
  try {
    return await fetch(request);
  } catch (error) {
    const cachedShell = await caches.match("/");
    if (cachedShell) return cachedShell;
    return new Response(
      `NASDrive could not load this page while the server was unavailable.\n\n${String(error)}`,
      {
        status: 503,
        statusText: "Service Unavailable",
        headers: { "Content-Type": "text/plain; charset=utf-8" },
      },
    );
  }
}
