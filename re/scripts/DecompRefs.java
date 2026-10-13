// Decompiles every function that references one of the given strings, or the given addresses.
// Args: 0x<address> (code), range:0x<a>-0x<b> (all functions), sym:<name> (callers of a symbol), ~<text> (strings containing text) or an exact string.
import ghidra.app.decompiler.DecompInterface;
import ghidra.app.decompiler.DecompileResults;
import ghidra.app.script.GhidraScript;
import ghidra.program.model.address.Address;
import ghidra.program.model.listing.Data;
import ghidra.program.model.listing.Function;
import ghidra.program.model.symbol.Reference;
import ghidra.program.model.symbol.Symbol;
import java.util.LinkedHashSet;
import java.util.Set;

public class DecompRefs extends GhidraScript {
    @Override
    public void run() throws Exception {
        Set<Function> funcs = new LinkedHashSet<>();
        for (String arg : getScriptArgs()) {
            if (arg.startsWith("0x")) {
                Function f = getFunctionContaining(toAddr(arg));
                if (f != null) funcs.add(f);
                continue;
            }
            if (arg.startsWith("range:")) {
                String[] bounds = arg.substring(6).split("-");
                Address end = toAddr(bounds[1]);
                for (Function f : currentProgram.getFunctionManager().getFunctions(toAddr(bounds[0]), true)) {
                    if (f.getEntryPoint().compareTo(end) > 0) break;
                    funcs.add(f);
                }
                continue;
            }
            if (arg.startsWith("sym:")) {
                for (Symbol s : currentProgram.getSymbolTable().getSymbols(arg.substring(4))) {
                    for (Reference r : getReferencesTo(s.getAddress())) {
                        Function f = getFunctionContaining(r.getFromAddress());
                        println("  " + arg + " ref from " + r.getFromAddress() + " in " + (f == null ? "?" : f.getName()));
                        if (f != null && !f.isThunk()) funcs.add(f);
                    }
                }
                continue;
            }
            for (Data d : currentProgram.getListing().getDefinedData(true)) {
                Object v = d.getValue();
                if (!(v instanceof String)) continue;
                String text = (String) v;
                if (arg.startsWith("~") ? !text.contains(arg.substring(1)) : !text.equals(arg)) continue;
                println("STRING " + arg + " @ " + d.getAddress());
                for (Reference r : getReferencesTo(d.getAddress())) {
                    Function f = getFunctionContaining(r.getFromAddress());
                    println("  ref from " + r.getFromAddress() + " in " + (f == null ? "?" : f.getName()));
                    if (f != null) funcs.add(f);
                }
            }
        }
        DecompInterface di = new DecompInterface();
        di.openProgram(currentProgram);
        for (Function f : funcs) {
            DecompileResults res = di.decompileFunction(f, 120, monitor);
            println("==== " + f.getName() + " @ " + f.getEntryPoint() + " callers:");
            for (Function c : f.getCallingFunctions(monitor)) println("  <- " + c.getName() + " @ " + c.getEntryPoint());
            println(res.decompileCompleted() ? res.getDecompiledFunction().getC() : "decompile failed: " + res.getErrorMessage());
        }
    }
}
