using System.Windows;

namespace SceneryAddonsBrowser
{
    public partial class ExitDownloadDialog : Window
    {
        public ExitChoice Result { get; private set; } = ExitChoice.Continue;

        public ExitDownloadDialog()
        {
            InitializeComponent();
        }

        private void Continue_Click(object sender, RoutedEventArgs e)
        {
            Result = ExitChoice.Continue;
            Close();
        }

        private void ExitResume_Click(object sender, RoutedEventArgs e)
        {
            Result = ExitChoice.ExitAndResume;
            Close();
        }

        private void CancelExit_Click(object sender, RoutedEventArgs e)
        {
            Result = ExitChoice.CancelAndExit;
            Close();
        }
    }

    public enum ExitChoice
    {
        Continue,
        ExitAndResume,
        CancelAndExit
    }
}
