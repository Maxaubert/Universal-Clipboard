# Spike S8: Microsoft Store and MSIX for the two-process Windows app

Date: 2026-11-13. Verdict: **GO**, with three changes to the plan (see the end).

Test layout: `spikes/s8-msix/layout/` (manifest and logos; the two exes were copies of `whoami.exe`, git-ignored).
`makeappx pack` (Windows SDK 10.0.26100.0, x64) built it: "Package creation succeeded", unsigned, 80 KB. Nothing was installed.

## Answers

| Question | Answer | Source |
|---|---|---|
| Can an individual open a Store developer account? | Yes. Pick "Individual developer" at storedeveloper.microsoft.com. Use a personal Microsoft account. ID check with a government ID and a selfie. | https://learn.microsoft.com/en-us/windows/apps/publish/partner-center/open-a-developer-account |
| Cost | No registration fee for both account types in the new flow (page dated 2026-10-29). Only the storedeveloper.microsoft.com entry gives the free flow; other entry points show the old flow. | same |
| Norway | The page lists no countries. I could not confirm Norway from the docs. Max should check on the sign-up page (the ID check will show it). | not verified |
| Store signing | Yes. "Code signing is handled free by the Store" for MSIX Store apps. No certificate for Store builds. Sideload builds need our own cert. | https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/packaging/ |
| Updates | Store-managed updates, and MSIX downloads only changed 64 KB blocks. Whether they are "silent" depends on the user's Store auto-update setting. Not tested. | https://learn.microsoft.com/en-us/windows/msix/overview |
| Clean uninstall | Yes: "clean uninstall with no leftover files or registry entries". | msix/overview |
| Startup task for the agent | `desktop:Extension Category="windows.startupTask"` with `desktop:StartupTask`. Validates. `Enabled="true"` turns it on when the app is first launched. Auto-start with no first launch needs `rescap5:ImmediateRegistration` plus a restricted custom capability. | https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop-startuptask |
| Firewall rule from the manifest | `desktop2:Extension Category="windows.firewallRules"` with `desktop2:FirewallRules Executable="agent.exe"` and `desktop2:Rule Direction="in" IPProtocol="TCP" Profile="private"`. Needs Windows 10 1703 or newer. **It must sit under `<Package><Extensions>`, not under `<Application>`**: makeappx rejected the first try with that exact message. Fixed and built. Also declare `privateNetworkClientServer`. | https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop2-firewallrules and https://learn.microsoft.com/en-us/uwp/schemas/appxpackage/uapmanifestschema/element-desktop2-rule |
| Full-trust background clipboard watcher | `runFullTrust` is a restricted capability: the Store needs approval in Partner Center on submit (no approval to sideload). A full-trust packaged app runs at medium integrity, outside the AppContainer. The docs I read set no clipboard limit for it. Not tested on a real install. | https://learn.microsoft.com/en-us/windows/uwp/packaging/app-capability-declarations |

## Two-process layout that validated

- `Settings` application: visible, `settings.exe`, opened by the user.
- `Agent` application: `AppListEntry="none"`, `agent.exe`, started by the startup task.
- Both use `EntryPoint="Windows.FullTrustApplication"` and capability `runFullTrust`.

## Changes to the plan

1. The agent does not start at sign-in until the user has launched the app once. Make first run open Settings once, or accept this.
2. Put firewall rules at package level. The rule is bound to `agent.exe` and the Private profile only.
3. Plan the `runFullTrust` approval request and the Norway account check before the first Store submission. Keep a non-Store build (self-signed MSIX or a plain installer) as a fallback.

## Not tested (needs an install, which this spike must not do)

Real firewall rule creation, startup task behavior at sign-in, clipboard access from the packaged agent, update timing.
