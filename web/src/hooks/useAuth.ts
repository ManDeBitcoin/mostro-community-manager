import { useCallback, useEffect, useState } from 'react';
import { LOGIN_REQUIRED_EVENT, api } from '../lib/api';

/** `password`: the panel asks for one. `external` or `open`: it does not. */
export type AuthSession = { mode: 'password' | 'external' | 'open'; authenticated: boolean };

export function useAuth() {
  const [session, setSession] = useState<AuthSession | null>(null);
  const [unreachable, setUnreachable] = useState(false);

  const check = useCallback(async () => {
    try {
      setSession(await api<AuthSession>('/api/auth/session'));
      setUnreachable(false);
    } catch {
      setUnreachable(true);
    }
  }, []);

  useEffect(() => {
    void check();
  }, [check]);

  // While the server does not answer, the panel shows that; once it does, the mode is known.
  useEffect(() => {
    if (!unreachable) return;
    const timer = setInterval(() => void check(), 10_000);
    return () => clearInterval(timer);
  }, [unreachable, check]);

  // A request answered with "log in" (the session expired, or the server
  // restarted and forgot it) brings the login page back.
  useEffect(() => {
    const onRequired = () => setSession({ mode: 'password', authenticated: false });
    window.addEventListener(LOGIN_REQUIRED_EVENT, onRequired);
    return () => window.removeEventListener(LOGIN_REQUIRED_EVENT, onRequired);
  }, []);

  const login = useCallback(async (password: string) => {
    await api('/api/auth/login', { method: 'POST', body: JSON.stringify({ password }) });
    setSession({ mode: 'password', authenticated: true });
  }, []);

  const logout = useCallback(async () => {
    await api('/api/auth/logout', { method: 'POST' });
    setSession((current) => current && { ...current, authenticated: current.mode !== 'password' });
  }, []);

  return { session, unreachable, login, logout };
}
