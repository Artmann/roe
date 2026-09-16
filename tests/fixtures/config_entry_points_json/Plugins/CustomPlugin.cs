namespace ConfigEntryPoints;

internal class CustomPlugin
{
    public void Run()
    {
        var helper = new PluginHelper();
        helper.Help();
    }
}
