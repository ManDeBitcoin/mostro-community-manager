import { useEffect, useState } from 'react';

export function LoginPage({ onLogin }: { onLogin: (password: string) => Promise<void> }) {
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    document.title = 'Iniciar sesión · Mostro Community Manager';
  }, []);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await onLogin(password);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'No se pudo iniciar sesión.');
      setPassword('');
      setBusy(false);
    }
  };

  return (
    <main className="login-page">
      <form className="login-card" onSubmit={submit}>
        <div className="login-brand">
          <img className="brand-mark" src="/brand-mark.png" alt="" aria-hidden="true" />
          <div className="brand-copy"><b>MOSTRO</b><span>COMMUNITY MANAGER</span></div>
        </div>
        <h1>Iniciar sesión</h1>
        <p>Escribe la contraseña del panel: la del archivo que indica <code>PANEL_PASSWORD_FILE</code> en la instalación.</p>
        <label className="field">
          <span className="field-label">Contraseña</span>
          <input type="password" autoComplete="current-password" autoFocus required value={password} onChange={(event) => setPassword(event.target.value)} />
        </label>
        {error && <div className="form-message error" role="alert">{error}</div>}
        <button type="submit" className="button button-primary" disabled={busy || !password}>
          {busy ? 'Comprobando…' : 'Entrar'}
        </button>
      </form>
    </main>
  );
}
