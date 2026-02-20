using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;
using SceneryAddonsBrowser.Logging;

namespace SceneryAddonsBrowser.Views
{
    public partial class UpdateDialog : Window
    {
        public bool ShouldUpdate { get; private set; }

        public UpdateDialog(string currentVersion, string newVersion, IEnumerable<string> changelog)
        {
            InitializeComponent();

            VersionText.Text = $"Current version: {currentVersion} → New version: {newVersion}";
            ChangelogList.ItemsSource = changelog?.ToList()
                ?? new List<string> { "fix bugs and performance." };

            AppLogger.Log($"UI: Update dialog shown. Current={currentVersion}, New={newVersion}");
        }

        private void Update_Click(object sender, RoutedEventArgs e)
        {
            ShouldUpdate = true;
            DialogResult = true;
            AppLogger.Log("UI: User chose to apply update from update dialog");
        }

        private void NotNow_Click(object sender, RoutedEventArgs e)
        {
            ShouldUpdate = false;
            DialogResult = false;
            AppLogger.Log("UI: User postponed update (Not Now)");
        }
    }
}
