using Velopack;

namespace SceneryAddonsBrowser.Models
{
    public class PendingUpdate
    {
        public UpdateInfo UpdateInfo { get; }
        public string CurrentVersion { get; }
        public string NewVersion { get; }
        public IReadOnlyList<string> Changelog { get; }

        public PendingUpdate(
            UpdateInfo updateInfo,
            string currentVersion,
            IReadOnlyList<string> changelog)
        {
            UpdateInfo = updateInfo;
            CurrentVersion = currentVersion;
            NewVersion = updateInfo.TargetFullRelease.Version.ToString();
            Changelog = changelog;
        }
    }
}
