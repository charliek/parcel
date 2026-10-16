import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import {
  isVerifiedReady,
  type PageMessage,
  parsePageMessage,
  SANDBOXED_ORIGIN,
  toPage,
  type ViewerMessage,
} from '@/lib/frame-protocol';

/** A verified conversation with the page in the frame (one per `ready`). */
export interface BridgeSession {
  /** The frame (URL + reload count) the page was loaded in. */
  key: string;
  /** The page's published path. */
  path: string;
  /** Bumped by every verified `ready`, so each new page is sent the current state. */
  epoch: number;
  /** The version of the grant the frame was loaded from. */
  version: number;
}

/** Messages the viewer handles once the bridge is live (`ready`/`bye` are the hook's). */
export type LivePageMessage = Exclude<PageMessage, { type: 'ready' } | { type: 'bye' }>;

interface FrameBridgeOptions {
  /** Changes whenever the iframe is remounted (its URL, or a reload). */
  frameKey: string;
  /** The capability token in the URL the frame was given (`contentToken`). */
  token: string | null;
  /** The version of the grant that URL came from. */
  version: number;
  /** Called with the session the message arrived in. */
  onMessage: (message: LivePageMessage, session: BridgeSession) => void;
}

/**
 * The viewer's end of the page-helper protocol, including the handshake
 * that decides whether the document in the frame is the parcel page we
 * loaded (see lib/frame-protocol.ts).
 *
 * The bridge is live only between a `ready` carrying the frame URL's token
 * and the next `load`, `bye` or remount. While it isn't, every page message
 * but `ready` and `bye` is dropped and `post` sends nothing; only `onLoad`
 * sends `hello`, which a parcel page answers with a fresh `ready`.
 *
 * `frameRef` goes on the content iframe. The frame is remounted on every
 * reload (it is keyed by URL and a nonce), so the listener is registered
 * once and checks the ref at event time.
 */
export function useFrameBridge({ frameKey, token, version, onMessage }: FrameBridgeOptions) {
  const frameRef = useRef<HTMLIFrameElement | null>(null);
  const [session, setSession] = useState<BridgeSession | null>(null);
  // What event handlers read: the latest props, and the session as of the
  // last event (state lags a render behind).
  const latest = useRef({
    frameKey,
    token,
    version,
    onMessage,
    session: null as BridgeSession | null,
  });
  const epochs = useRef(0);
  // Layout phase, so a message handled right after a commit already sees
  // that render's frame, token and version together.
  useLayoutEffect(() => {
    latest.current.frameKey = frameKey;
    latest.current.token = token;
    latest.current.version = version;
    latest.current.onMessage = onMessage;
  });

  useEffect(() => {
    const liveSession = () => {
      const s = latest.current.session;
      return s !== null && s.key === latest.current.frameKey ? s : null;
    };
    const listener = (event: MessageEvent) => {
      const frame = frameRef.current?.contentWindow;
      // Only our iframe, and only as an opaque-origin sandboxed document.
      if (!frame || event.source !== frame || event.origin !== SANDBOXED_ORIGIN) return;
      const message = parsePageMessage(event.data);
      if (!message) return;
      if (message.type === 'bye') {
        // The page is going away. Accepted even when not live: it can only
        // turn the bridge off.
        latest.current.session = null;
        setSession(null);
        return;
      }
      if (message.type === 'ready') {
        if (!isVerifiedReady(message, latest.current.token)) return;
        epochs.current += 1;
        // Key, token and version all come from the same render.
        const next = {
          key: latest.current.frameKey,
          path: message.path,
          epoch: epochs.current,
          version: latest.current.version,
        };
        latest.current.session = next;
        setSession(next);
        return;
      }
      const session = liveSession();
      if (session) latest.current.onMessage(message, session);
    };
    window.addEventListener('message', listener);
    return () => window.removeEventListener('message', listener);
  }, []);

  /**
   * The sandboxed page has an opaque origin, so no targetOrigin but '*' can
   * reach it. That is acceptable because these messages carry nothing
   * secret (anchors taken from the parcel's own page, comment numbers and
   * flags; never comment text, author names or tokens), and because they
   * go out only while the bridge is live, i.e. to the page we loaded.
   */
  const send = useCallback((message: ViewerMessage) => {
    frameRef.current?.contentWindow?.postMessage(message, '*');
  }, []);

  const post = useCallback(
    (message: ViewerMessage) => {
      const s = latest.current.session;
      if (s !== null && s.key === latest.current.frameKey) send(message);
    },
    [send],
  );

  /** Every iframe load, including navigations inside the frame. */
  const onLoad = useCallback(() => {
    // Whatever the frame held before is gone. A page posts its own `ready`
    // before `load` fires, so that one is discarded here; a parcel page
    // answers this `hello` with another, which re-establishes the bridge.
    latest.current.session = null;
    setSession(null);
    send(toPage.hello());
  }, [send]);

  // A remounted frame (new URL or reload) starts out not live.
  const live = session && session.key === frameKey ? session : null;
  return { frameRef, live, post, onLoad };
}
