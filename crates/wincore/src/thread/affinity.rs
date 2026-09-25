//! Logical CPUs and affinity masks, in Windows processor groups.

/// `GROUP_AFFINITY`, field for field.
#[repr(C)]
#[derive(Default)]
pub(crate) struct C_GroupAffinity {
    pub(crate) mask: usize,
    pub(crate) group: u16,
    pub(crate) reserved: [u16; 3],
}
impl C_GroupAffinity {
    pub(crate) fn new(cpu: ProcessorId) -> Self {
        Self {
            mask: 1usize << cpu.number,
            group: cpu.group,
            reserved: [0u16; 3],
        }
    }
}
impl From<C_GroupAffinity> for ThreadAffinity {
    fn from(value: C_GroupAffinity) -> Self {
        Self {
            mask: value.mask,
            group: value.group,
        }
    }
}

/// The set of CPUs a thread may run on, within one processor group.
///
/// A single-bit mask means a pinned thread; a mask with every CPU of the group
/// means it is free to migrate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadAffinity {
    /// One bit per logical CPU of the group.
    pub mask: usize,
    /// The processor group.
    pub group: u16,
}
impl ThreadAffinity {
    pub(crate) fn single_cpu(cpu: ProcessorId) -> Self {
        Self {
            mask: 1usize << cpu.number,
            group: cpu.group,
        }
    }

    /// Enumerates the CPUs the mask allows.
    pub fn cpus(&self) -> impl Iterator<Item = ProcessorId> {
        (0..usize::BITS)
            .filter(|&i| self.mask & (1 << i) != 0)
            .map(|i| ProcessorId {
                number: i as u8,
                group: self.group,
            })
    }
}

/// `group:cpu` for a pinned thread, `group: {a,b,...,z}` for several CPUs.
impl std::fmt::Display for ThreadAffinity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.mask.count_ones() == 1 {
            return write!(f, "{}:{}", self.group, self.mask.trailing_zeros());
        }
        write!(f, "{}: {{", self.group)?;
        for (i, cpu) in self.cpus().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{}", cpu.number)?;
        }
        f.write_str("}")
    }
}

/// `PROCESSOR_NUMBER`, field for field.
#[repr(C)]
#[derive(Default)]
pub(crate) struct C_ProcessorNumber {
    pub(crate) group: u16,
    pub(crate) number: u8,
    pub(crate) reserved: u8,
}

impl From<C_ProcessorNumber> for ProcessorId {
    fn from(value: C_ProcessorNumber) -> Self {
        Self {
            number: value.number,
            group: value.group,
        }
    }
}

/// Identifies a single logical CPU by its number within its processor group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessorId {
    pub(crate) number: u8,
    pub(crate) group: u16,
}
impl ProcessorId {
    /// The CPU `number` of processor group `group`. `None` if `number` does
    /// not fit in an affinity mask (64 CPUs per group on x86_64).
    pub const fn try_new(number: u32, group: u16) -> Option<Self> {
        if number >= usize::BITS {
            return None;
        }

        Some(Self {
            number: number as u8,
            group,
        })
    }
    /// The CPU's number within its group.
    pub fn number(&self) -> u8 {
        self.number
    }
    /// The processor group.
    pub fn group(&self) -> u16 {
        self.group
    }
}

/// `group:number`.
impl std::fmt::Display for ProcessorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.group, self.number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_and_multi_cpu_masks_display_differently() {
        let pinned = ThreadAffinity {
            mask: 1 << 3,
            group: 0,
        };
        let three = ThreadAffinity {
            mask: 0b111,
            group: 0,
        };
        assert_eq!(pinned.to_string(), "0:3");
        assert_eq!(three.to_string(), "0: {0,1,2}");
    }
}
