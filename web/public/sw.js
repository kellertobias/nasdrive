const CACHE_NAME = "nasfiles-shell-v2";
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
    event.respondWith(cacheFirstWithRefresh(event.request));
    return;
  }

  event.respondWith(
    caches.match(event.request).then((cached) => cached || fetch(event.request)),
  );
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
    pathname.startsWith("/assets/") ||
    pathname.startsWith("/pwa/") ||
    pathname === "/manifest.webmanifest" ||
    pathname === "/favicon.svg"
  );
}

async function cacheFirstWithRefresh(request) {
  const cached = await caches.match(request);
  const refresh = fetch(request).then(async (response) => {
    if (response.ok) {
      const cache = await caches.open(CACHE_NAME);
      await cache.put(request, response.clone());
    }
    return response;
  });

  if (cached) {
    refresh.catch(() => undefined);
    return cached;
  }

  return refresh;
}

async function navigationFallback(request) {
  try {
    return await fetch(request);
  } catch (error) {
    const cachedShell = await caches.match("/");
    if (cachedShell) return cachedShell;
    return new Response(
      `Nasfiles could not load this page while the server was unavailable.\n\n${String(error)}`,
      {
        status: 503,
        statusText: "Service Unavailable",
        headers: { "Content-Type": "text/plain; charset=utf-8" },
      },
    );
  }
}
