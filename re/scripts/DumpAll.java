// Decompiles every function into one C file for grepping. Arg: output path.
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.listing.Function;
import java.io.PrintWriter;

public class DumpAll extends GhidraScript {
    @Override
    public void run() throws Exception {
        DecompInterface di = new DecompInterface();
        di.openProgram(currentProgram);
        try (PrintWriter out = new PrintWriter(getScriptArgs()[0])) {
            for (Function f : currentProgram.getFunctionManager().getFunctions(true)) {
                if (monitor.isCancelled()) break;
                DecompileResults res = di.decompileFunction(f, 60, monitor);
                out.println("// ==== " + f.getName() + " @ " + f.getEntryPoint());
                out.println(res.decompileCompleted() ? res.getDecompiledFunction().getC() : "// decompile failed: " + res.getErrorMessage());
            }
        }
    }
}
