namespace Lib;

// Both methods are over the config's maxComplexity of 2, but only Known is
// recorded in the config's baseline — so only New may be reported.
public class Widget
{
    public void Known(bool a, bool b)
    {
        if (a)
        {
            DoA();
        }
        else if (b)
        {
            DoB();
        }
    }

    public void New(bool a, bool b, bool c)
    {
        if (a)
        {
            DoA();
        }

        if (b)
        {
            DoB();
        }

        if (c)
        {
            DoA();
        }
    }

    private void DoA() { }

    private void DoB() { }
}
