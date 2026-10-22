import re
src = open('src/fs/blueprint.rs').read().split('\n')
L = lambda a, b: src[a-1:b]  # inclusive 1-based
head = L(1, 542) + ['}', '']  # through end of expand fn; close impl
regions_impl = L(543, 742)    # volume..table_children
records_impl = L(743, 1087)   # record_size..chain_cell
free_regions = L(1089, 1143)  # ascii, cells, journal_descriptor
free_records = L(1144, 1219)  # var_slot_children, bits_children
gaps = L(1220, 1246)
tests = L(1248, 1532)

def vis(lines):
    out = []
    for l in lines:
        l = re.sub(r'^    fn ', '    pub(super) fn ', l)
        l = re.sub(r'^    const fn ', '    pub(super) const fn ', l)
        l = re.sub(r'^fn ', 'pub(super) fn ', l)
        out.append(l)
    return out

uses = '''use super::{Node, Part, SlotOwners};
'''
open('src/fs/blueprint/regions.rs', 'w').write(
    '//! Regions of the volume: superblock, journal and chunk tables.\n\n' + uses + '\n'
    'impl<D: BlockDevice> Fs<D> {\n' + '\n'.join(vis(regions_impl)) + '\n}\n\n' + '\n'.join(vis(free_regions)) + '\n')
open('src/fs/blueprint/records.rs', 'w').write(
    '//! Chunks, records, variable-size objects, usage sets and file extents.\n\n' + uses + '\n'
    'impl<D: BlockDevice> Fs<D> {\n' + '\n'.join(vis(records_impl)) + '\n}\n\n' + '\n'.join(vis(free_records)) + '\n')
t = tests[:]
assert t[0] == 'mod tests {', t[0]
assert t[-1] == '}'
body = [l[4:] if l.startswith('    ') else l for l in t[1:-1]]
open('src/fs/blueprint/tests.rs', 'w').write('\n'.join(body) + '\n')
mod = head + gaps + ['#[cfg(test)]', 'mod tests;', '']
open('src/fs/blueprint/mod.rs', 'w').write('\n'.join(mod))
