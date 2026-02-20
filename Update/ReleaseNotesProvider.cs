using System;
using System.IO;

namespace SceneryAddonsBrowser.Update
{
    public static class ReleaseNotesProvider
    {
        public static string Load(string version)
        {
            try
            {
                var baseDir = AppContext.BaseDirectory;
                var path = Path.Combine(baseDir, "ReleaseNotes", $"{version}.html");

                if (File.Exists(path))
                    return File.ReadAllText(path);
            }
            catch
            {

            }

            return "<p>No release notes were provided.</p>";
        }
    }
}
