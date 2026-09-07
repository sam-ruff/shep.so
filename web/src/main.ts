import { BrowserSettings, Workspace } from "./model";
import { BrowserStore } from "./storage";
import { GatewayRepository } from "./provider";
import { mount } from "./ui";
import { readSession, showLogin, signOut } from "./auth";
async function start() {
  try {
    const session = await readSession();
    if (!session) {
      showLogin();
      return;
    }
    const repository = new GatewayRepository(
      session,
      await BrowserStore.open(session.user_id),
    );
    await repository.load();
    const workspace = new Workspace(
      repository,
      new BrowserSettings(`shep.preferences.v1.${session.user_id}`),
    );
    workspace.error = repository.warning;
    mount(workspace, {
      email: session.email,
      signOut: () => {
        repository.forgetPasswords();
        void signOut(session).catch(() => {
          workspace.error = "Could not sign out. Retry.";
          workspace.changed();
        });
      },
    });
    const timer = setInterval(() => {
      if (
        !document.hidden &&
        !workspace.syncing &&
        repository.accounts.some((a) => repository.connected(a.id))
      )
        void workspace.refresh();
    }, 15_000);
    addEventListener(
      "pagehide",
      () => {
        clearInterval(timer);
        repository.forgetPasswords();
      },
      { once: true },
    );
  } catch (error) {
    showLogin(
      error instanceof Error
        ? error.message
        : "The beta service is unavailable.",
    );
  }
}
void start();
