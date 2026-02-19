using System.Collections.Generic;
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

            VersionText.Text = $"Current: {currentVersion} → New: {newVersion}";
            ChangelogList.ItemsSource = changelog;
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
