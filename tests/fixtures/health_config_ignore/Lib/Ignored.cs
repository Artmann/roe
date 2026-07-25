namespace Lib;

// Matched by the config's ignore glob. Both the findings in here and the
// cycle entirely contained in here must be dropped.
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
