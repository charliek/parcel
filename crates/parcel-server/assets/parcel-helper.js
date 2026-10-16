/*
 * Parcel page helper.
 *
 * The content origin adds this script to every HTML page it serves (see
 * `content::helper`). It runs inside the parcel's sandbox with exactly the
 * page's own powers — no cookies, no tokens beyond the capability already in
 * the page's URL — and gives the page four things:
 *
 * 1. Storage that doesn't throw. A sandboxed page has an opaque origin, so
 *    `localStorage`, `sessionStorage` and `document.cookie` throw on access.
 *    They are replaced with in-memory stand-ins (lost on reload) so pages
 *    written for a normal origin keep working.
 * 2. `window.parcel.data`: JSON data kept on the server between visits,
 *    readable by every viewer and writable by the parcel's owner.
 * 3. Comment mode, driven by the Parcel viewer that frames the page: it
 *    reports where the user clicked and draws comment pins. Messages go only
 *    to and from the app origin, and carry anchors and numbers — never
 *    comment text. The one credential, the page's own capability token, is
 *    sent in `ready` to the app origin that minted it, so the viewer can tell
 *    its parcel page from any other document the frame is navigated to; the
 *    page says `bye` when it unloads.
 * 4. Links to other sites open in a new tab. In the viewer's frame, a plain
 *    click on a link to another origin (with no target, or `_self`) gets
 *    `target="_blank" rel="noopener noreferrer"` before the browser follows
 *    it. Most sites refuse to be framed anyway, and the viewer's frame should
 *    only ever hold the parcel. Links within the parcel's origin, and
 *    `javascript:`, `blob:`, `data:` and other non-http(s) URLs, are left alone.
 */
(() => {
  'use strict';
  if (window.__parcelHelper) return;
  window.__parcelHelper = true;

  // Taken before any page script runs, so a page library that wraps fetch
  // (error trackers, analytics) never sees a data request's capability URL.
  const nativeFetch = window.fetch.bind(window);

  // Replaced by the server with the app origin, as a JSON string.
  const APP_ORIGIN = "__PARCEL_APP_ORIGIN__";

  // ---------------------------------------------------------------- location

  // Content URLs are /v/{token}/{path}; the token is
  // base64url("id.version.expires.scope") + "." + signature.
  const match = /^\/v\/([^/]+)\/(.*)$/.exec(location.pathname);
  const token = match ? match[1] : null;
  const decode = (s) => {
    try {
      return decodeURIComponent(s);
    } catch (_) {
      return s;
    }
  };
  const rawPath = match ? match[2].split('/').map(decode).join('/') : '';
  // The server answers a directory URL with its index.html.
  const pagePath = rawPath === '' || rawPath.endsWith('/') ? `${rawPath}index.html` : rawPath;
  const claims = { id: null, version: null, owner: false };
  if (token) {
    try {
      const payload = atob(token.split('.')[0].replace(/-/g, '+').replace(/_/g, '/')).split('.');
      claims.id = payload[0];
      claims.version = Number(payload[1]);
      claims.owner = payload[3] === 'o';
    } catch (_) {
      // Not a parcel URL after all; data calls will fail cleanly.
    }
  }

  // ----------------------------------------------------------------- storage

  function memoryStorage() {
    const items = new Map();
    const api = {
      get length() {
        return items.size;
      },
      key: (i) => {
        const k = Array.from(items.keys())[i];
        return k === undefined ? null : k;
      },
      getItem: (k) => {
        const key = String(k);
        return items.has(key) ? items.get(key) : null;
      },
      setItem: (k, v) => {
        items.set(String(k), String(v));
      },
      removeItem: (k) => {
        items.delete(String(k));
      },
      clear: () => {
        items.clear();
      },
    };
    // A Proxy so `localStorage.foo = 'x'` and `Object.keys(localStorage)`
    // behave like the real thing.
    return new Proxy(api, {
      get: (t, p) => (typeof p === 'symbol' || p in t ? t[p] : items.get(p)),
      set: (t, p, v) => {
        if (typeof p === 'symbol' || p in t) return false;
        items.set(p, String(v));
        return true;
      },
      has: (t, p) => p in t || items.has(p),
      deleteProperty: (_t, p) => items.delete(p) || true,
      ownKeys: () => Array.from(items.keys()),
      getOwnPropertyDescriptor: (_t, p) =>
        items.has(p)
          ? { value: items.get(p), writable: true, enumerable: true, configurable: true }
          : undefined,
    });
  }

  for (const name of ['localStorage', 'sessionStorage']) {
    let works = false;
    try {
      works = Boolean(window[name]);
    } catch (_) {
      works = false;
    }
    if (!works) {
      const store = memoryStorage();
      try {
        Object.defineProperty(window, name, {
          configurable: true,
          enumerable: true,
          get: () => store,
        });
      } catch (_) {
        // Leave it throwing; nothing else to do.
      }
    }
  }

  try {
    void document.cookie;
  } catch (_) {
    const jar = new Map();
    try {
      Object.defineProperty(document, 'cookie', {
        configurable: true,
        get: () => Array.from(jar, ([k, v]) => `${k}=${v}`).join('; '),
        set: (raw) => {
          const text = String(raw);
          const first = text.split(';')[0];
          const eq = first.indexOf('=');
          const key = (eq < 0 ? '' : first.slice(0, eq)).trim();
          const value = (eq < 0 ? first : first.slice(eq + 1)).trim();
          const expires = /;\s*expires=([^;]+)/i.exec(text);
          const maxAge = /;\s*max-age=(-?\d+)/i.exec(text);
          const gone =
            (maxAge && Number(maxAge[1]) <= 0) ||
            (expires && Date.parse(expires[1]) <= Date.now());
          if (gone) jar.delete(key);
          else jar.set(key, value);
        },
      });
    } catch (_) {
      // Leave it throwing.
    }
  }

  // -------------------------------------------------------------------- data

  // Absolute, on the content host: a page's <base href> must not send the
  // capability anywhere else. (`location` reflects the URL even though the
  // sandboxed document's origin is opaque.)
  const dataBase = token ? `${location.protocol}//${location.host}/d/${token}` : null;
  const enc = (name) => encodeURIComponent(String(name));

  function dataError(message, status) {
    const err = new Error(message);
    err.status = status;
    return err;
  }

  async function request(method, suffix, body) {
    if (!dataBase) throw dataError('parcel data is only available on a parcel page', 0);
    const init = { method, cache: 'no-store' };
    if (body !== undefined) {
      init.headers = { 'content-type': 'application/json' };
      init.body = JSON.stringify(body);
    }
    const res = await nativeFetch(dataBase + suffix, init);
    if (res.status === 204) return null;
    let json = null;
    try {
      json = await res.json();
    } catch (_) {
      json = null;
    }
    if (!res.ok) {
      const message =
        (json && json.error && json.error.message) || `parcel data request failed (${res.status})`;
      throw dataError(message, res.status);
    }
    return json;
  }

  function writable() {
    if (!claims.owner) {
      throw dataError("only the parcel's owner can change its data; viewers can read it", 403);
    }
  }

  function itemPath(collection, key) {
    return `/${enc(collection)}/${enc(key)}`;
  }

  const data = Object.freeze({
    /** The stored value, or undefined when there is none. */
    async get(collection, key) {
      try {
        return (await request('GET', itemPath(collection, key))).value;
      } catch (err) {
        if (err.status === 404) return undefined;
        throw err;
      }
    },
    /** Every item in a collection: [{key, value, updated_at, …}], by key. */
    async list(collection) {
      const items = [];
      let after = null;
      do {
        const query = after ? `?limit=1000&after=${enc(after)}` : '?limit=1000';
        const page = await request('GET', `/${enc(collection)}${query}`);
        items.push(...page.items);
        after = page.next_after || null;
      } while (after);
      return items;
    },
    /** Replace (or create) an item; resolves to the stored value. */
    async set(collection, key, value) {
      writable();
      if (value === undefined) throw new TypeError('parcel.data.set needs a value');
      return (await request('PUT', itemPath(collection, key), value)).value;
    },
    /** Merge an object into an item (null removes a field); resolves to the result. */
    async update(collection, key, patch) {
      writable();
      return (await request('PATCH', itemPath(collection, key), patch)).value;
    },
    async delete(collection, key) {
      writable();
      await request('DELETE', itemPath(collection, key));
    },
    /** Delete every item in a collection. */
    async clear(collection) {
      writable();
      await request('DELETE', `/${enc(collection)}`);
    },
    /** [{name, count, usage_bytes, updated_at}] */
    async collections() {
      return (await request('GET', '')).collections;
    },
    /**
     * Call `callback({revision})` whenever the parcel's data changes (checked
     * every few seconds while the page is visible). Returns an unsubscribe
     * function. Your own writes trigger it too.
     */
    subscribe(callback) {
      let last = null;
      let timer = null;
      let stopped = false;
      const tick = async () => {
        if (stopped) return;
        if (document.visibilityState !== 'hidden') {
          try {
            const summary = await request('GET', '');
            if (last !== null && summary.revision !== last) callback({ revision: summary.revision });
            last = summary.revision;
          } catch (_) {
            // Try again next tick.
          }
        }
        if (!stopped) timer = setTimeout(tick, 4000);
      };
      tick();
      return () => {
        stopped = true;
        clearTimeout(timer);
      };
    },
  });

  try {
    // Configurable and writable, so a page's own `let parcel`/`var parcel`
    // still works (and simply hides this).
    Object.defineProperty(window, 'parcel', {
      configurable: true,
      writable: true,
      enumerable: false,
      value: Object.freeze({
        id: claims.id,
        version: claims.version,
        path: pagePath,
        canWrite: claims.owner,
        data,
      }),
    });
  } catch (_) {
    // A page that froze `window` doesn't get the API.
  }

  // ----------------------------------------------------------- comment mode

  if (window.parent === window) return;

  const post = (type, extra) => {
    try {
      window.parent.postMessage(Object.assign({ source: 'parcel-helper', type }, extra), APP_ORIGIN);
    } catch (_) {
      // The parent isn't the app; nothing to talk to.
    }
  };

  let commenting = false;
  let pins = [];
  let draft = null;
  let active = null;
  let pulseUntil = 0;
  let host = null;
  let hover = null;
  let layer = null;
  let lastReport = '';
  const found = new Map();

  // Whitespace collapsed and control characters dropped (the server refuses
  // them in anchors).
  const norm = (s) =>
    String(s || '')
      .slice(0, 2000)
      .replace(/\s+/g, ' ')
      .replace(/[\u0000-\u001f\u007f-\u009f]/g, '')
      .trim();
  const textOf = (el) => norm(el.textContent).slice(0, 200);
  const clamp = (n) => Math.min(1, Math.max(0, n));
  const round = (n) => Math.round(n * 1000) / 1000;
  const isOurs = (node) => Boolean(host && node && (node === host || host.contains(node)));

  function ensureOverlay() {
    if (host && host.isConnected) return;
    host = document.createElement('parcel-overlay');
    host.style.cssText =
      'all:initial;position:fixed;inset:0;pointer-events:none;z-index:2147483647;';
    const root = host.attachShadow({ mode: 'closed' });
    root.innerHTML = `<style>
      .hover{position:fixed;display:none;box-sizing:border-box;border:2px solid #4f46e5;
        background:rgba(79,70,229,.08);border-radius:3px;pointer-events:none}
      .pin{position:fixed;box-sizing:border-box;min-width:26px;height:26px;padding:0 7px;
        transform:translate(-3px,-23px);border:2px solid #fff;border-radius:13px 13px 13px 3px;
        background:#4f46e5;color:#fff;font:600 12px/22px system-ui,sans-serif;text-align:center;
        box-shadow:0 1px 4px rgba(0,0,0,.35);cursor:pointer;pointer-events:auto;
        transition:transform .15s}
      .pin:hover{transform:translate(-3px,-23px) scale(1.12)}
      .pin.resolved{background:#6b7280}
      .pin.active{box-shadow:0 0 0 3px rgba(79,70,229,.45),0 1px 4px rgba(0,0,0,.35)}
      .pin.pulse{animation:pulse 1s ease-out 2}
      .pin.draft{background:#fff;color:#4f46e5;border-color:#4f46e5;border-style:dashed;cursor:default}
      @keyframes pulse{50%{transform:translate(-3px,-23px) scale(1.35)}}
    </style><div class="hover"></div><div class="layer"></div>`;
    hover = root.querySelector('.hover');
    layer = root.querySelector('.layer');
    document.documentElement.appendChild(host);
  }

  function selectorFor(el) {
    const parts = [];
    let node = el;
    while (node && node.nodeType === 1 && node !== document.documentElement) {
      const tag = node.localName;
      if (node.id) {
        const sel = `${tag}#${CSS.escape(node.id)}`;
        let unique = false;
        try {
          unique = document.querySelectorAll(sel).length === 1;
        } catch (_) {
          unique = false;
        }
        if (unique) {
          parts.unshift(sel);
          return parts.join(' > ');
        }
      }
      let index = 1;
      let same = 0;
      for (let s = node.parentElement && node.parentElement.firstElementChild; s; s = s.nextElementSibling) {
        if (s.localName !== tag) continue;
        same += 1;
        if (s === node) index = same;
      }
      parts.unshift(same > 1 ? `${tag}:nth-of-type(${index})` : tag);
      node = node.parentElement;
    }
    return parts.length ? `html > ${parts.join(' > ')}` : 'html';
  }

  function anchorFor(el, clientX, clientY) {
    const r = el.getBoundingClientRect();
    const anchor = { path: pagePath };
    const selector = selectorFor(el);
    if (selector.length <= 1000) anchor.selector = selector;
    const text = textOf(el);
    if (text) anchor.text = text;
    anchor.x = round(r.width ? clamp((clientX - r.left) / r.width) : 0.5);
    anchor.y = round(r.height ? clamp((clientY - r.top) / r.height) : 0.5);
    return anchor;
  }

  // Find an anchor's element on this page: its selector if the text still
  // matches, else the innermost element of the same tag with that text, else
  // the selector's element anyway.
  function locate(anchor) {
    let bySelector = null;
    if (anchor.selector) {
      try {
        bySelector = document.querySelector(anchor.selector);
      } catch (_) {
        bySelector = null;
      }
      if (isOurs(bySelector)) bySelector = null;
    }
    const probe = anchor.text ? norm(anchor.text).slice(0, 60) : '';
    if (!probe || (bySelector && textOf(bySelector).startsWith(probe))) return bySelector;
    const last = anchor.selector ? anchor.selector.split('>').pop().trim() : '';
    const tag = /^[a-z][a-z0-9-]*/i.exec(last);
    if (!tag) return bySelector;
    let best = null;
    const candidates = document.getElementsByTagName(tag[0]);
    for (let i = 0; i < candidates.length && i < 5000; i++) {
      const c = candidates[i];
      if (textOf(c).startsWith(probe) && (!best || best.contains(c))) best = c;
    }
    return best || bySelector;
  }

  function elementFor(key, anchor) {
    const cached = found.get(key);
    if (cached && cached.isConnected) return cached;
    const el = locate(anchor);
    if (el) found.set(key, el);
    else found.delete(key);
    return el;
  }

  function position(key, anchor) {
    const el = elementFor(key, anchor);
    if (!el) return null;
    const r = el.getBoundingClientRect();
    const x = anchor.x === undefined ? 0.5 : anchor.x;
    const y = anchor.y === undefined ? 0.5 : anchor.y;
    return { left: r.left + x * r.width, top: r.top + y * r.height, el };
  }

  function pinElement(label, pos, classes) {
    const pin = document.createElement('div');
    pin.className = `pin ${classes}`;
    pin.textContent = label;
    pin.style.left = `${pos.left}px`;
    pin.style.top = `${pos.top}px`;
    return pin;
  }

  function render() {
    scheduled = false;
    const wanted = pins.filter((p) => p.anchor && p.anchor.path === pagePath);
    if (!wanted.length && !draft && !commenting && !host) return;
    ensureOverlay();
    layer.textContent = '';
    const located = [];
    const missing = [];
    for (const p of wanted) {
      const pos = position(p.number, p.anchor);
      if (!pos) {
        missing.push(p.number);
        continue;
      }
      located.push(p.number);
      const isActive = p.number === active;
      const pulsing = isActive && Date.now() < pulseUntil;
      const classes = `${p.resolved ? 'resolved' : ''} ${isActive ? 'active' : ''} ${pulsing ? 'pulse' : ''}`;
      const pin = pinElement(String(p.number), pos, classes);
      pin.setAttribute('role', 'button');
      pin.setAttribute('aria-label', `Comment ${p.number}`);
      pin.addEventListener('click', (e) => {
        e.preventDefault();
        e.stopPropagation();
        active = p.number;
        schedule();
        post('pin-click', { number: p.number });
      });
      layer.appendChild(pin);
    }
    if (draft) {
      const pos = position('draft', draft);
      if (pos) layer.appendChild(pinElement('+', pos, 'draft'));
    }
    const report = `${located.join(',')}|${missing.join(',')}`;
    if (report !== lastReport) {
      lastReport = report;
      post('located', { found: located, missing });
    }
  }

  let scheduled = false;
  function schedule() {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(render);
  }

  function setCommenting(on) {
    commenting = on;
    document.documentElement.classList.toggle('parcel-commenting', on);
    if (on && !document.getElementById('parcel-commenting-style')) {
      const style = document.createElement('style');
      style.id = 'parcel-commenting-style';
      style.textContent =
        'html.parcel-commenting,html.parcel-commenting *{cursor:crosshair!important}';
      (document.head || document.documentElement).appendChild(style);
    }
    if (!on && hover) hover.style.display = 'none';
    schedule();
  }

  // While commenting, the page must not react to clicks: these run first
  // (capture phase on window, registered before any page script).
  const swallow = (e) => {
    if (!commenting || isOurs(e.target)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
  };
  for (const type of ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'dblclick', 'auxclick', 'contextmenu']) {
    window.addEventListener(type, swallow, true);
  }
  window.addEventListener(
    'click',
    (e) => {
      if (!commenting || isOurs(e.target)) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      const el = e.target && e.target.nodeType === 1 ? e.target : document.body;
      if (!el) return;
      draft = anchorFor(el, e.clientX, e.clientY);
      found.delete('draft');
      schedule();
      post('anchor', { anchor: draft });
    },
    true,
  );
  // A link to another origin opens in a new tab rather than in the viewer's
  // frame (see the header). Capture phase, so the target is set before the
  // click's default action reads it; comment mode swallows clicks above.
  function linkOf(e) {
    const path = typeof e.composedPath === 'function' ? e.composedPath() : [e.target];
    for (const node of path) {
      if (
        (node instanceof HTMLAnchorElement || node instanceof HTMLAreaElement) &&
        node.hasAttribute('href')
      ) {
        return node;
      }
    }
    return null;
  }
  window.addEventListener(
    'click',
    (e) => {
      if (commenting || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const link = linkOf(e);
      if (!link) return;
      const target = (link.getAttribute('target') || '').trim().toLowerCase();
      if (target && target !== '_self') return;
      let url;
      try {
        url = new URL(link.href);
      } catch (_) {
        return;
      }
      // javascript:, blob:, data:, mailto: and the like keep their default.
      if (url.protocol !== 'http:' && url.protocol !== 'https:') return;
      // `location.origin` is the URL's origin (the content host), even though
      // the sandboxed document's own origin is opaque.
      if (url.origin === location.origin) return;
      link.setAttribute('target', '_blank');
      const rel = new Set((link.getAttribute('rel') || '').split(/\s+/).filter(Boolean));
      rel.add('noopener');
      rel.add('noreferrer');
      link.setAttribute('rel', Array.from(rel).join(' '));
    },
    true,
  );
  window.addEventListener(
    'mousemove',
    (e) => {
      if (!commenting || !hover) return;
      const el = e.target;
      if (!el || el.nodeType !== 1 || isOurs(el)) {
        hover.style.display = 'none';
        return;
      }
      const r = el.getBoundingClientRect();
      Object.assign(hover.style, {
        display: 'block',
        left: `${r.left}px`,
        top: `${r.top}px`,
        width: `${r.width}px`,
        height: `${r.height}px`,
      });
    },
    { capture: true, passive: true },
  );
  window.addEventListener(
    'keydown',
    (e) => {
      if (commenting && e.key === 'Escape') post('escape', {});
    },
    true,
  );

  // Pins follow their elements through scrolling (of any container),
  // resizes and DOM changes.
  window.addEventListener('scroll', schedule, { capture: true, passive: true });
  window.addEventListener('resize', schedule, { passive: true });
  new MutationObserver(() => {
    found.clear();
    schedule();
  }).observe(document.documentElement, {
    childList: true,
    subtree: true,
    characterData: true,
    attributes: true,
  });
  setInterval(() => {
    if (pins.length || draft) schedule();
  }, 1000);

  window.addEventListener('message', (e) => {
    if (e.source !== window.parent || e.origin !== APP_ORIGIN) return;
    const msg = e.data;
    if (!msg || msg.source !== 'parcel-viewer') return;
    switch (msg.type) {
      case 'hello':
        sendReady();
        break;
      case 'comment-mode':
        setCommenting(Boolean(msg.enabled));
        break;
      case 'pins': {
        pins = Array.isArray(msg.pins) ? msg.pins : [];
        // The viewer owns which thread is selected; this keeps the
        // highlight across page reloads and for threads it just created.
        const flagged = pins.find((p) => p && p.active === true);
        active = flagged ? flagged.number : null;
        lastReport = '';
        schedule();
        break;
      }
      case 'draft':
        draft = msg.anchor && msg.anchor.path === pagePath ? msg.anchor : null;
        found.delete('draft');
        schedule();
        break;
      case 'focus': {
        const p = pins.find((x) => x.number === msg.number);
        active = typeof msg.number === 'number' ? msg.number : null;
        if (p && p.anchor && p.anchor.path === pagePath) {
          const el = elementFor(p.number, p.anchor);
          if (el) el.scrollIntoView({ block: 'center', behavior: 'smooth' });
          pulseUntil = Date.now() + 2000;
        }
        schedule();
        break;
      }
      default:
        break;
    }
  });

  // The token is what the viewer checks before it talks to this frame: only
  // the page it loaded knows it. `post` sends only to the app origin, which
  // minted it.
  function sendReady() {
    post('ready', { path: pagePath, title: document.title, version: claims.version, token });
  }
  sendReady();
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', sendReady, { once: true });
  }
  // Leaving: the viewer stops sending until the next page proves itself.
  window.addEventListener('pagehide', () => post('bye', {}));
  window.addEventListener('pageshow', (e) => {
    if (e.persisted) sendReady();
  });
})();
