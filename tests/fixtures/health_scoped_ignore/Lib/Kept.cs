namespace Lib;

// Neither this file nor its cycle is covered by the config's ignore glob, so
// both must survive the filtering that drops Ignored.cs.
public class KeptWidget
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

public class KeptAlpha
{
    public KeptBeta Beta;
}

public class KeptBeta
{
    public KeptAlpha Alpha;
}
