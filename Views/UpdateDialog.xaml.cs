using System;
using System.Collections.Generic;
using System.Linq;
using System.Windows;

namespace SceneryAddonsBrowser.Views
{
    public partial class UpdateDialog : Window
    {
        public bool ShouldUpdate { get; private set; }

        public UpdateDialog(
            string currentVersion,
            string newVersion,
            IEnumerable<string> changelog)
        {
            InitializeComponent();

            if (changelog == null)
                throw new InvalidOperationException("Changelog is NULL");

            var list = changelog.ToList();

            if (list.Count == 0)
                list.Add("No release notes were provided for this update.");

            VersionText.Text =
                $"Current version: {currentVersion} → New version: {newVersion}";

            ChangelogList.ItemsSource = list;
        }

        private void Update_Click(object sender, RoutedEventArgs e)
        {
            ShouldUpdate = true;
            DialogResult = true;
        }

        private void NotNow_Click(object sender, RoutedEventArgs e)
        {
            ShouldUpdate = false;
            DialogResult = false;
        }
    }
}
