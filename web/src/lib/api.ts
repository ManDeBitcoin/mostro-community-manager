export const copyToClipboard = (text: string, onSuccess: () => void) => {
  if (navigator.clipboard && window.isSecureContext) {
    navigator.clipboard.writeText(text).then(onSuccess);
  } else {
    const textArea = document.createElement("textarea");
    textArea.value = text;
    textArea.style.position = "absolute";
    textArea.style.left = "-999999px";
    document.body.prepend(textArea);
    textArea.select();
    try {
      document.execCommand('copy');
      onSuccess();
    } catch (error) {
      console.error(error);
    } finally {
      textArea.remove();
    }
  }
};

/** Fired when the server answers that the panel needs a login (see useAuth). */
export const LOGIN_REQUIRED_EVENT = 'mcm:login-required';

export async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    ...init,
    headers: {
      'X-Requested-With': 'mostro-community',
      ...(init?.headers || {}),
      ...(init?.body ? { 'Content-Type': 'application/json' } : {})
    }
  });
  const data = await response.json().catch(() => ({}));
  if (response.status === 401 && data?.login_required) window.dispatchEvent(new Event(LOGIN_REQUIRED_EVENT));
  if (!response.ok) throw new Error(data?.error || `Error del servidor (${response.status})`);
  return data as T;
}
