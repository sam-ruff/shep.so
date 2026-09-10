import { BrowserSettings, Workspace } from "./model";
import { BrowserStore } from "./storage";
import { GatewayRepository } from "./provider";
import { mount } from "./ui";
import { readSession, showLogin, signOut } from "./auth";
import { GatewayProfileGoogle } from "./profile_google";
import { WorkerHistoryPort } from "./profile_history";
import { ProfileSettingsStore } from "./profile_settings";
import { ProfilesUI } from "./profiles_ui";
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
    const settings = new ProfileSettingsStore(
      new BrowserSettings(`shep.preferences.v1.${session.user_id}`),
      `shep.profile-preferences.v1.${session.user_id}`,
    );
    const workspace = new Workspace(repository, settings);
    workspace.error = repository.warning;
    const port = new WorkerHistoryPort(
      () =>
        new Worker(new URL("./profile_history_worker.ts", import.meta.url), {
          type: "module",
        }),
    );
    let profiles: ProfilesUI | undefined;
    const view = mount(
      workspace,
      {
        email: session.email,
        signOut: () => {
          repository.forgetPasswords();
          void signOut(session).catch(() => {
            workspace.error = "Could not sign out. Retry.";
            workspace.changed();
          });
        },
      },
      (profiles = new ProfilesUI({
        workspace,
        repository,
        api: new GatewayProfileGoogle(session),
        settings,
        port,
        identity: session.user_id,
        openPreferences: () => view.openPreferences(),
      })),
    );
    void profiles.start();
    const timer = setInterval(() => {
      if (
        !document.hidden &&
        !workspace.syncing &&
        repository.accounts.some((a) => repository.connected(a.id))
      )
        void workspace.refresh();
    }, 15_000);
    addEventListener("pageshow", (event) => {
      if (event.persisted) location.reload();
    });
    addEventListener(
      "pagehide",
      () => {
        clearInterval(timer);
        repository.forgetPasswords();
        repository.stopMailbox();
        profiles?.dispose();
        port.dispose();
        workspace.dispose();
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
