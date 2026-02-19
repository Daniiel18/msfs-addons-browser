using System.Collections.Generic;
using Velopack;

namespace SceneryAddonsBrowser.Models
{
    public class PendingUpdate
    {
        public UpdateInfo UpdateInfo { get; }
        public string CurrentVersion { get; }
        public IEnumerable<string> Changelog { get; }

        public PendingUpdate(
            UpdateInfo updateInfo,
            string currentVersion,
            IEnumerable<string> changelog)
        {
            UpdateInfo = updateInfo;
            CurrentVersion = currentVersion;
            Changelog = changelog;
        }
    }
}
