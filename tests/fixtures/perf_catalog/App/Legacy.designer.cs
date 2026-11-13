using System.Threading.Tasks;

namespace App;

/// Generated code nobody maintains by hand. Its `.Result` is the most severe
/// finding in the catalog and must still produce nothing.
public class LegacyBridge
{
    public string Fetch(Task<string> pending)
    {
        return pending.Result;
    }
}
