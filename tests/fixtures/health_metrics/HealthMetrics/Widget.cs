namespace HealthMetrics;

public class Widget
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

    public void ManyParams(int a, int b, int c, int d, int e, int f)
    {
    }

    private void DoA() { }

    private void DoB() { }
}
