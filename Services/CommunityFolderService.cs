using System.IO;
using System.Text.RegularExpressions;

namespace SceneryAddonsBrowser.Services
{
    public class CommunityFolderService
    {
        public string? GetCommunityPath()
        {
            var userCfg = FindUserCfgOpt();
            if (userCfg == null)
                return null;

            var content = File.ReadAllText(userCfg);

            var match = Regex.Match(
                content,
                @"InstalledPackagesPath\s+""(.+?)""",
                RegexOptions.IgnoreCase);

            if (!match.Success)
                return null;

            var packagesPath = match.Groups[1].Value.Trim();

            var community = Path.Combine(packagesPath, "Community");

            return Directory.Exists(community)
                ? community
                : null;
        }

        private string? FindUserCfgOpt()
        {
            var paths = new[]
            {
                Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
                    "Packages",
                    "Microsoft.FlightSimulator_8wekyb3d8bbwe",
                    "LocalCache",
                    "UserCfg.opt"),

                Path.Combine(
                    Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
                    "Microsoft Flight Simulator",
                    "UserCfg.opt")
            };

            foreach (var path in paths)
            {
                if (File.Exists(path))
                    return path;
            }

            return null;
        }
    }
}
