export interface Session {
  email: string;
  csrf: string;
  user_id: string;
}
export async function readSession(): Promise<Session | null> {
  const response = await fetch("/api/session", {
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error",
  });
  if (response.status === 401) return null;
  if (!response.ok)
    throw new Error("The beta service is unavailable. Try again shortly.");
  const value: unknown = await response.json();
  if (
    typeof value !== "object" ||
    value === null ||
    !("email" in value) ||
    !("csrf" in value) ||
    !("user_id" in value) ||
    typeof value.user_id !== "string" ||
    !/^[A-Za-z0-9_-]{43}$/.test(value.user_id) ||
    typeof value.email !== "string" ||
    typeof value.csrf !== "string" ||
    value.csrf.length !== 43
  ) {
    throw new Error(
      "The beta service returned an invalid session. Sign in again.",
    );
  }
  return { email: value.email, csrf: value.csrf, user_id: value.user_id };
}
export async function signOut(session: Session): Promise<void> {
  const response = await fetch("/api/logout", {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    redirect: "error",
    headers: { "x-shep-csrf": session.csrf },
  });
  if (!response.ok && response.status !== 401)
    throw new Error("Could not sign out. Retry.");
  location.assign("/beta");
}
export function showLogin(message?: string) {
  const root = document.querySelector("#app")!;
  root.className = "login-root";
  const panel = document.createElement("main");
  panel.className = "login-panel";
  const title = document.createElement("h1");
  title.textContent = "Shep private beta";
  const text = document.createElement("p");
  text.textContent = "Sign in with an invited Google account to continue.";
  const link = document.createElement("a");
  link.className = "button primary";
  link.href = "/auth/start";
  link.textContent = "Continue with Google";
  panel.append(title, text, link);
  if (message) {
    const error = document.createElement("p");
    error.role = "alert";
    error.textContent = message;
    panel.append(error);
  }
  root.replaceChildren(panel);
}
