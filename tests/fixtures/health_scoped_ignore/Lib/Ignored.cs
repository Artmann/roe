namespace Lib;

// Matched by the config's `health.ignore` glob. The findings in here and the
// cycle entirely contained in here must be dropped from the health report —
// while this file's dead-code finding survives, because the scoped glob
// applies to health alone.
public class IgnoredWidget
{
    public void Branchy(bool a, bool b, bool c)
    {
        if (a)
        {
            DoA();
        }
        else if (b && c)
        {
            DoB();
        }
    }

    private void DoA() { }

    private void DoB() { }
}

public class IgnoredGamma
{
    public IgnoredDelta Delta;
}

public class IgnoredDelta
{
    public IgnoredGamma Gamma;
}

internal class IgnoredLedger
{
    public void DoNothing()
    {
    }
}
